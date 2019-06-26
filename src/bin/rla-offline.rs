#![deny(unused_must_use)]
#![allow(
    clippy::collapsible_if,
    clippy::needless_range_loop,
    clippy::useless_let_if_seq
)]

extern crate brotli;
extern crate env_logger;
extern crate failure;
#[macro_use]
extern crate log;
extern crate rust_log_analyzer as rla;
extern crate walkdir;

use std::path::PathBuf;
use structopt::StructOpt;

mod offline;
mod util;

#[derive(StructOpt)]
#[structopt(
    name = "Rust Log Analyzer Offline Tools",
    about = "A collection of tools to run the log analyzer without starting a server."
)]
enum Cli {
    #[structopt(
        name = "cat",
        about = "Read, and optionally process, a previously downloaded log file, then dump it to stdout."
    )]
    Cat {
        #[structopt(
            short = "s",
            long = "strip-control",
            help = "Removes all ASCII control characters, except newlines, before dumping."
        )]
        strip_control: bool,
        #[structopt(
            short = "d",
            long = "decode-utf8",
            help = "Lossily decode as UTF-8 before dumping."
        )]
        decode_utf8: bool,
        #[structopt(help = "The log file to read and dump.")]
        input: PathBuf,
    },

    #[structopt(name = "learn", about = "Learn from previously downloaded log files.")]
    Learn {
        #[structopt(
            short = "i",
            long = "index-file",
            help = "The index file to read / write. An existing index file is updated."
        )]
        index_file: PathBuf,
        #[structopt(
            short = "m",
            long = "multiplier",
            default_value = "1",
            help = "A multiplier to apply when learning."
        )]
        multiplier: u32,
        #[structopt(
            help = "The log files to learn from.\nDirectories are traversed recursively. Hidden files are ignore."
        )]
        logs: Vec<PathBuf>,
    },

    #[structopt(
        name = "extract-dir",
        about = "Extract potential error messages from all log files in a directory, writing the results to a different directory."
    )]
    ExtractDir {
        #[structopt(
            short = "i",
            long = "index-file",
            help = "The index file to read / write."
        )]
        index_file: PathBuf,
        #[structopt(
            short = "s",
            long = "source",
            help = "The directory in which to (non-recursively) look for log files. Hidden files are ignored."
        )]
        source: PathBuf,
        #[structopt(
            short = "d",
            long = "destination",
            help = "The directory in which to write the results. All non-hidden will be deleted from the directory."
        )]
        dest: PathBuf,
    },

    #[structopt(
        name = "extract-one",
        about = "Extract a potential error message from a single log file."
    )]
    ExtractOne {
        #[structopt(
            short = "i",
            long = "index-file",
            help = "The index file to read / write."
        )]
        index_file: PathBuf,
        #[structopt(help = "The log file to analyze.")]
        log: PathBuf,
    },

    #[structopt(name = "travis-dl", about = "Download build logs from travis")]
    TravisDl {
        #[structopt(short = "o", long = "output", help = "Log output directory.")]
        output: PathBuf,
        #[structopt(
            short = "q",
            long = "query",
            help = "Travis /builds filter query parameters."
        )]
        query: String,
        #[structopt(short = "c", long = "count", help = "Number of _builds_ to process.")]
        count: u32,
        #[structopt(
            short = "s",
            long = "skip",
            default_value = "0",
            help = "Number of _builds_ to skip."
        )]
        skip: u32,
        #[structopt(
            short = "j",
            long = "job-filters",
            use_delimiter = true,
            help = "Comma-separated lists of job states to filter by."
        )]
        job_filter: Vec<String>,
    },
}

fn main() {
    util::run(|| match Cli::from_args() {
        Cli::Cat {
            strip_control,
            decode_utf8,
            input,
        } => offline::dl::cat(&input, strip_control, decode_utf8),
        Cli::Learn {
            index_file,
            multiplier,
            logs,
        } => offline::learn(&index_file, &logs, multiplier),
        Cli::ExtractDir {
            index_file,
            source,
            dest,
        } => offline::extract::dir(&index_file, &source, &dest),
        Cli::ExtractOne { index_file, log } => offline::extract::one(&index_file, &log),
        Cli::TravisDl {
            output,
            query,
            count,
            skip,
            job_filter,
        } => offline::dl::travis(&output, &query, count, skip, &job_filter),
    });
}
