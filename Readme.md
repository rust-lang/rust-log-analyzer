# Rust Log Analyzer

The **Rust Log Analyzer** (**RLA**) is a tool that analyzes the CI build logs of the [rust-lang/rust](https://github.com/rust-lang/rust) repository with the purpose of automatically extracting error messages from failed builds.

This repository contains three components:

* The **`rust_log_analyzer`** library, which contains the analysis logic.
* The **`rla-offline`** binary, a collection of various tools to run the analyzer off-line (see the output of `rla-offline --help` for available commands).
* The **`rla-server`** binary, a web server which receives GitHub webhooks and automatically posts analysis results to the Rust repository.

## Running RLA

RLA uses the `log` crate to write all output. By default, anything logged at `INFO` or higher will be printed. You can change this behavior by setting the `RLA_LOG` environment variable, using the syntax specified by the `env_logger` crate.

### Secrets

To run commands which access online resources, you have to provide the required authentication information in environment variables:

* For *GitHub*, set `GITHUB_USER` to your GitHub user name and `GITHUB_TOKEN` to a [personal access token](https://github.com/settings/tokens) with at least "repo" scope.
* For *Travis*, set `TRAVIS_API_KEY` to the API key of [your Travis account](https://travis-ci.org/profile).

### Bootstrapping an index file

To initialize a new index file, perform the following steps:

1. Download some successful build logs using the `rla-offline dl` command.
    * It is recommended that you run in `release` mode.
    * I'm still gathering data, but you should probably have well over 1000 log files (this does not mean over 1000 *builds*, since one builds consists of dozens of jobs)
    * Example command: `rla-offline dl --ci travis -c 40 --branch auto --passed -o data/training`
2. Train on the downloaded logs using the `rla-offline learn command`.
    * Example command: `rla-offline learn -i demo.idx data/training`

### Analyzing a specific log

1. Download the log file you want to analyze using either the `rla-offline dl` command or manually from [travis-ci.org](https:/travis-ci.org).
    * All tools will automatically decompress files ending in `.brotli`, or assume uncompressed data otherwise.
2. Use the `rla-offline extract-one` command analyze the log file.
    * Example command: `rla-offline extract-one -i demo.idx my-log.txt`

### Evaluating quality while developing

*Note: This process will / should be integrated as regression tests.*

1. Download, or otherwise curate, a set of log files you want to evaluate against.
    * Example command: `rla-offline dl --ci travis -c 50 --failed -o data/failed`
    * *Note: Eventually, a set of test log files will be provided in the repository.*
2. Use the `rla-offline extract-dir` command to analyze all the log files and write the results to a separate directory.
    * You can (temporarily) check the result directory in to the repository to see diffs.
    * Example command: `rla-offline extract-dir -i demo.idx -s data/failed -d data/err`
    * *Note: Eventually, the expected results for the test log files will be provided in the repository and used as regression tests.*
