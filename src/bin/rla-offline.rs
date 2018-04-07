extern crate brotli;
#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
extern crate rust_log_analyzer as rla;

use clap::{Arg, SubCommand};

mod offline;
mod util;

static APP_NAME: &str = "Rust Log Analyzer Offline Tools";
static ABOUT: &str = "A collection of tools to run the log analyzer without starting a server.";

fn main() {
    util::run(APP_NAME, ABOUT, |app| {
        let matches = app
            .subcommand(SubCommand::with_name("travis-dl")
                .about("Download build logs from travis")
                .arg(Arg::from_usage("-o, --output=<DIRECTORY> 'Log output directory.'").required(true))
                .arg(Arg::from_usage("-q, --query=<FILTER> 'Travis /builds filter query parameters.'").required(true))
                .arg(Arg::from_usage("-c, --count=<INT> 'Number of _builds_ to process.'").required(true))
                .arg(Arg::from_usage("-s, --skip=<INT> 'Number of builds to skip.'").required(false))
                .arg(Arg::from_usage("-j, --job-filter=<STATES> 'Comma-separated lists of job states to filter by.").required(false)))
            .get_matches();

        match matches.subcommand() {
            ("travis-dl", Some(args)) => offline::dl::travis(args),
            _ => bail!("No command provided. Use --help to list available commands."),
        }
    });
}
