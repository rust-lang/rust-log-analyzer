#![deny(unused_must_use)]
#![allow(
    clippy::collapsible_if,
    clippy::needless_range_loop,
    clippy::useless_let_if_seq
)]

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

use std::process;
use std::rc::Rc;
use std::sync;
use std::thread;
use structopt::StructOpt;

mod server;
mod util;

#[derive(StructOpt)]
#[structopt(
    name = "Rust Log Analyzer WebHook Server",
    about = "A http server that listens for GitHub webhooks and posts comments with potential causes on failed builds."
)]
struct Cli {
    #[structopt(
        short = "p",
        long = "port",
        default_value = "8080",
        help = "The port to listen on for HTTP connections."
    )]
    port: u16,
    #[structopt(
        short = "b",
        long = "bind",
        default_value = "127.0.0.1",
        help = "The address to bind."
    )]
    bind: std::net::IpAddr,
    #[structopt(
        short = "i",
        long = "index-file",
        help = "The index file to read / write. An existing index file is updated."
    )]
    index_file: std::path::PathBuf,
    #[structopt(
        long = "debug-post",
        help = "Post all comments to the given issue instead of the actual PR. Format: \"user/repo#id\""
    )]
    debug_post: Option<String>,
    #[structopt(
        long = "webhook-verify",
        help = "If enabled, web hooks that cannot be verified are rejected."
    )]
    webhook_verify: bool,
}

fn main() {
    util::run(|| {
        let args = Cli::from_args();

        let addr = std::net::SocketAddr::new(args.bind, args.port);

        let (queue_send, queue_recv) = sync::mpsc::channel();

        let service = Rc::new(server::RlaService::new(args.webhook_verify, queue_send)?);

        let mut worker = server::Worker::new(args.index_file, args.debug_post, queue_recv)?;

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
