use clap;
use env_logger;
use log;
use rla;
use std::env;
use std::process;

pub fn run<F: FnOnce(clap::App) -> rla::Result<()>>(app_name: &str, about: &str, f: F) {
    let mut log_builder = env_logger::Builder::new();

    if let Ok(s) = env::var("RLA_LOG") {
        log_builder.parse(&s);
    } else {
        log_builder.filter(None, log::LevelFilter::Info);
    }

    if let Ok(s) = env::var("RLA_LOG_STYLE") {
        log_builder.parse_write_style(&s);
    }

    log_builder.init();

    let app = clap::App::new(app_name)
        .version(crate_version!())
        .author(crate_authors!())
        .about(about);

    if let Err(e) = f(app) {
        error!("{}", e);
        process::exit(1);
    }
}
