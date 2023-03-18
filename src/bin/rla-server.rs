#![deny(unused_must_use)]
#![allow(
    clippy::collapsible_if,
    clippy::needless_range_loop,
    clippy::useless_let_if_seq
)]

#[macro_use]
extern crate failure;
extern crate futures;
extern crate hyper;
#[macro_use]
extern crate tracing;
extern crate crossbeam;
extern crate regex;
extern crate rust_log_analyzer as rla;
extern crate serde_json;

use clap::Parser;
use std::process;
use std::sync::Arc;
use std::thread;

mod server;
mod util;

#[derive(Parser, Debug)]
#[command(
    name = "Rust Log Analyzer WebHook Server",
    about = "A http server that listens for GitHub webhooks and posts comments with potential causes on failed builds."
)]
struct Cli {
    #[arg(
        short = 'p',
        long = "port",
        default_value = "8080",
        help = "The port to listen on for HTTP connections."
    )]
    port: u16,
    #[arg(
        short = 'b',
        long = "bind",
        default_value = "127.0.0.1",
        help = "The address to bind."
    )]
    bind: std::net::IpAddr,
    #[arg(
        short = 'i',
        long = "index-file",
        help = "The index file to read / write. An existing index file is updated."
    )]
    index_file: std::path::PathBuf,
    #[arg(
        long = "debug-post",
        help = "Post all comments to the given issue instead of the actual PR. Format: \"user/repo#id\""
    )]
    debug_post: Option<String>,
    #[arg(
        long = "webhook-verify",
        help = "If enabled, web hooks that cannot be verified are rejected."
    )]
    webhook_verify: bool,
    #[arg(long = "ci", help = "CI platform to interact with.")]
    ci: util::CliCiPlatform,
    #[arg(long = "repo", help = "Repository to interact with.")]
    repo: String,
    #[arg(
        long = "secondary-repo",
        help = "Secondary repositories to listen for builds.",
        required = false
    )]
    secondary_repos: Vec<String>,
    #[arg(
        long = "query-builds-from-primary-repo",
        help = "Always query builds from the primary repo instead of the repo receiving them."
    )]
    query_builds_from_primary_repo: bool,
}

#[test]
fn verify_cli() {
    use clap::CommandFactory;
    Cli::command().debug_assert()
}

fn main() {
    dotenv::dotenv().ok();
    util::run(|| {
        let args = Cli::parse();

        let addr = std::net::SocketAddr::new(args.bind, args.port);

        let (queue_send, queue_recv) = crossbeam::channel::unbounded();

        let service = Arc::new(server::RlaService::new(args.webhook_verify, queue_send)?);

        let mut worker = server::Worker::new(
            args.index_file,
            args.debug_post,
            queue_recv,
            args.ci.get()?,
            args.repo,
            args.secondary_repos,
            args.query_builds_from_primary_repo,
        )?;

        thread::spawn(move || {
            if let Err(e) = worker.main() {
                error!("Worker failed, exiting: {}", e);
                process::exit(1);
            }

            info!("Work finished, exiting.");

            process::exit(0);
        });

        tokio::runtime::Runtime::new()?.block_on(async move {
            let s = service.clone();
            hyper::server::Server::bind(&addr)
                .serve(hyper::service::make_service_fn(move |_| {
                    let s = s.clone();
                    async move {
                        Ok::<_, hyper::Error>(hyper::service::service_fn(move |req| {
                            let s = s.clone();
                            async move { s.call(req).await }
                        }))
                    }
                }))
                .await
        })?;

        Ok(())
    });
}
