# Rust Log Analyzer

The **Rust Log Analyzer** (**RLA**) is a tool that analyzes the CI build logs of the [rust-lang/rust](https://github.com/rust-lang/rust) repository with the purpose of automatically extracting error messages from failed builds.

This repository contains three components:

* The **`rust_log_analyzer`** library, which contains the analysis logic.
* The **`rla-offline`** binary, a collection of various tools to run the analyzer off-line (see the output of `rla-offline --help` for available commands).
* The **`rla-server`** binary, a web server which receives GitHub webhooks and automatically posts analysis results to the Rust repository.
