#![deny(unused_must_use)]
#![allow(
    clippy::collapsible_if,
    clippy::needless_range_loop,
    clippy::useless_let_if_seq
)]

#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate failure;
extern crate futures;
#[macro_use]
extern crate hyper;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate regex;
extern crate rust_log_analyzer as rla;
extern crate serde_json;

use clap::Arg;
use std::process;
use std::rc::Rc;
use std::sync;
use std::thread;

mod server;
mod util;

static APP_NAME: &str = "Rust Log Analyzer WebHook Server";
static ABOUT: &str = "A http server that listens for GitHub webhooks and posts comments with potential causes on failed builds.";

fn main() {
    util::run(APP_NAME, ABOUT, |app| {
        let matches = app
            .arg(Arg::from_usage("-p, --port=[INT] 'The port to listen on for HTTP connections.'")
                .default_value("8080"))
            .arg(Arg::from_usage("-b, --bind=[ADDRESS] 'The address to bind.'")
                .default_value("127.0.0.1"))
            .arg(Arg::from_usage("-i, --index-file=<FILE> 'The index file to read / write.'"))
            .arg(Arg::from_usage("--debug-post=[GITHUB_ISSUE] 'Post all comments to the given issue instead of the actual PR. Format: \"user/repo#id\"'"))
            .arg(Arg::from_usage("--webhook-verify 'If enabled, web hooks that cannot be verified are rejected.'"))
            .get_matches();

        let addr = format!(
            "{}:{}",
            matches.value_of("bind").unwrap(),
            matches.value_of("port").unwrap()
        )
        .parse()?;

        let (queue_send, queue_recv) = sync::mpsc::channel();

        let service = Rc::new(server::RlaService::new(&matches, queue_send)?);

        let mut worker = server::Worker::new(&matches, queue_recv)?;

        thread::spawn(move || {
            if let Err(e) = worker.main() {
                error!("Worker failed, exiting: {}", e);
                process::exit(1);
            }

            info!("Work finished, exiting.");

            process::exit(0);
        });

        let server = hyper::server::Http::new().bind(&addr, move || Ok(service.clone()))?;

        server.run()?;

        Ok(())
    });
}
