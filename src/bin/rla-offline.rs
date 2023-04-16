#![deny(unused_must_use)]
#![allow(
    clippy::collapsible_if,
    clippy::needless_range_loop,
    clippy::useless_let_if_seq
)]

extern crate brotli;
#[macro_use]
extern crate tracing;
extern crate rust_log_analyzer as rla;
extern crate walkdir;

use clap::Parser;
use rla::index::IndexStorage;
use std::path::PathBuf;

mod offline;
mod util;

#[derive(Debug, Parser)]
#[command(
    name = "Rust Log Analyzer Offline Tools",
    about = "A collection of tools to run the log analyzer without starting a server."
)]
enum Cli {
    #[command(
        name = "cat",
        about = "Read, and optionally process, a previously downloaded log file, then dump it to stdout."
    )]
    Cat {
        #[arg(
            short = 's',
            long = "strip-control",
            help = "Removes all ASCII control characters, except newlines, before dumping."
        )]
        strip_control: bool,
        #[arg(
            short = 'd',
            long = "decode-utf8",
            help = "Lossily decode as UTF-8 before dumping."
        )]
        decode_utf8: bool,
        #[arg(help = "The log file to read and dump.")]
        input: PathBuf,
    },

    #[command(name = "learn", about = "Learn from previously downloaded log files.")]
    Learn {
        #[arg(long = "ci", help = "CI platform to download from.")]
        ci: util::CliCiPlatform,
        #[arg(
            short = 'i',
            long = "index-file",
            help = "The index file to read / write. An existing index file is updated."
        )]
        index_file: IndexStorage,
        #[arg(
            short = 'm',
            long = "multiplier",
            default_value = "1",
            help = "A multiplier to apply when learning."
        )]
        multiplier: u32,
        #[arg(
            help = "The log files to learn from.\nDirectories are traversed recursively. Hidden files are ignore."
        )]
        logs: Vec<PathBuf>,
    },

    #[command(
        name = "extract-dir",
        about = "Extract potential error messages from all log files in a directory, writing the results to a different directory."
    )]
    ExtractDir {
        #[arg(long = "ci", help = "CI platform to download from.")]
        ci: util::CliCiPlatform,
        #[arg(
            short = 'i',
            long = "index-file",
            help = "The index file to read / write."
        )]
        index_file: IndexStorage,
        #[arg(
            short = 's',
            long = "source",
            help = "The directory in which to (non-recursively) look for log files. Hidden files are ignored."
        )]
        source: PathBuf,
        #[arg(
            short = 'd',
            long = "destination",
            help = "The directory in which to write the results. All non-hidden will be deleted from the directory."
        )]
        dest: PathBuf,
    },

    #[command(
        name = "extract-one",
        about = "Extract a potential error message from a single log file."
    )]
    ExtractOne {
        #[arg(long = "ci", help = "CI platform to download from.")]
        ci: util::CliCiPlatform,
        #[arg(
            short = 'i',
            long = "index-file",
            help = "The index file to read / write."
        )]
        index_file: IndexStorage,
        #[arg(help = "The log file to analyze.")]
        log: PathBuf,
    },

    #[command(name = "dl", about = "Download build logs from the CI platform.")]
    Dl {
        #[arg(long = "ci", help = "CI platform to download from.")]
        ci: util::CliCiPlatform,
        #[arg(long = "repo", help = "Repository to download from.")]
        repo: String,
        #[arg(short = 'o', long = "output", help = "Log output directory.")]
        output: PathBuf,
        #[arg(short = 'c', long = "count", help = "Number of _builds_ to process.")]
        count: u32,
        #[arg(
            short = 's',
            long = "skip",
            default_value = "0",
            help = "Number of _builds_ to skip."
        )]
        skip: u32,
        #[arg(short = 'b', long = "branch", help = "Branches to filter by.")]
        branches: Vec<String>,
        #[arg(long = "passed", help = "Only download passed builds and jobs.")]
        passed: bool,
        #[arg(long = "failed", help = "Only download failed builds and jobs.")]
        failed: bool,
    },
}

#[test]
fn verify_cli() {
    use clap::CommandFactory;
    Cli::command().debug_assert()
}

fn main() {
    dotenv::dotenv().ok();
    util::run(|| match Cli::parse() {
        Cli::Cat {
            strip_control,
            decode_utf8,
            input,
        } => offline::dl::cat(&input, strip_control, decode_utf8),
        Cli::Learn {
            ci,
            index_file,
            multiplier,
            logs,
        } => offline::learn(ci.get()?.as_ref(), &index_file, &logs, multiplier),
        Cli::ExtractDir {
            ci,
            index_file,
            source,
            dest,
        } => offline::extract::dir(ci.get()?.as_ref(), &index_file, &source, &dest),
        Cli::ExtractOne {
            ci,
            index_file,
            log,
        } => offline::extract::one(ci.get()?.as_ref(), &index_file, &log),
        Cli::Dl {
            ci,
            repo,
            output,
            count,
            skip,
            branches,
            passed,
            failed,
        } => offline::dl::download(
            ci.get()?.as_ref(),
            &repo,
            &output,
            count,
            skip,
            &branches,
            passed,
            failed,
        ),
    });
}
