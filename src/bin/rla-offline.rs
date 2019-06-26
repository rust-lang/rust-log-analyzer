#![deny(unused_must_use)]
#![cfg_attr(
    feature = "cargo-clippy",
    allow(collapsible_if, needless_range_loop, useless_let_if_seq)
)]

extern crate brotli;
#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
extern crate rust_log_analyzer as rla;
extern crate walkdir;

use clap::{Arg, SubCommand};

mod offline;
mod util;

static APP_NAME: &str = "Rust Log Analyzer Offline Tools";
static ABOUT: &str = "A collection of tools to run the log analyzer without starting a server.";

fn main() {
    util::run(APP_NAME, ABOUT, |app| {
        let matches = app
            .subcommand(SubCommand::with_name("cat")
                .about("Read, and optionally process, a previously downloaded log file, then dump it to stdout.")
                .arg(Arg::from_usage("-s, --strip-control 'Removes all ASCII control characters, except newlines, before dumping.'"))
                .arg(Arg::from_usage("-d, --decode-utf8 'Lossily decode as UTF-8 before dumping.'"))
                .arg(Arg::from_usage("<input> 'The log file to read and dump.'")))
            .subcommand(SubCommand::with_name("learn")
                .about("Learn from previously downloaded log files.")
                .arg(Arg::from_usage("-i, --index-file=<FILE> 'The index file to read / write. An existing index file is updated.'"))
                .arg(Arg::from_usage("-m, --multiplier=[INT] 'A multiplier to apply when learning.'")
                    .default_value("1"))
                .arg(Arg::from_usage("<logs>... 'The log files to learn from.\nDirectories are traversed recursively. Hidden files are ignore.'")))
            .subcommand(SubCommand::with_name("extract-dir")
                .about("Extract potential error messages from all log files in a directory, writing the results to a different directory.")
                .arg(Arg::from_usage("-i, --index-file=<FILE> 'The index file to read / write.'"))
                .arg(Arg::from_usage("-s, --source=<DIR> 'The directory in which to (non-recursively) look for log files. Hidden files are ignored.'"))
                .arg(Arg::from_usage("-d, --destination=<DIR> 'The directory in which to write the results. All non-hidden will be deleted from the directory.'")))
            .subcommand(SubCommand::with_name("extract-one")
                .about("Extract a potential error message from a single log file.")
                .arg(Arg::from_usage("-i, --index-file=<FILE> 'The index file to read / write.'"))
                .arg(Arg::from_usage("<log> 'The log file to analyze.'")))
            .subcommand(SubCommand::with_name("travis-dl")
                .about("Download build logs from travis")
                .arg(Arg::from_usage("-o, --output=<DIRECTORY> 'Log output directory.'"))
                .arg(Arg::from_usage("-q, --query=<FILTER> 'Travis /builds filter query parameters.'"))
                .arg(Arg::from_usage("-c, --count=<INT> 'Number of _builds_ to process.'"))
                .arg(Arg::from_usage("-s, --skip=[INT] 'Number of builds to skip.'")
                    .default_value("0"))
                .arg(Arg::from_usage("-j, --job-filter=[STATES]... 'Comma-separated lists of job states to filter by.")
                    .use_delimiter(true)
                    .possible_values(offline::dl::TRAVIS_JOB_STATES)))
            .get_matches();

        match matches.subcommand() {
            ("cat", Some(args)) => offline::dl::cat(args),
            ("extract-dir", Some(args)) => offline::extract::dir(args),
            ("extract-one", Some(args)) => offline::extract::one(args),
            ("learn", Some(args)) => offline::learn(args),
            ("travis-dl", Some(args)) => offline::dl::travis(args),
            _ => bail!("No command provided. Use --help to list available commands."),
        }
    });
}
