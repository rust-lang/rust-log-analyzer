#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
extern crate rust_log_analyzer as rla;

mod util;

static APP_NAME: &str = "Rust Log Analyzer Offline Tools";
static ABOUT: &str = "A collection of tools to run the log analyzer without starting a server.";

fn main() {
    util::run(APP_NAME, ABOUT, |app| {
        let matches = app.get_matches();

        match matches.subcommand() {
            _ => bail!("No command provided. Use --help to list available commands."),
        }

        Ok(())
    });
}
