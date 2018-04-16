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

    log_and_exit_error(|| f(app));
}

pub fn log_and_exit_error<F: FnOnce() -> rla::Result<()>>(f: F) {
    if let Err(e) = f() {
        error!("{}\n\n{}", e, e.backtrace());
        process::exit(1);
    }
}
