use crate::rla;
use failure::ResultExt;
use std::process;

pub(crate) enum CliCiPlatform {
    Azure,
    Actions,
}

impl CliCiPlatform {
    pub(crate) fn get(&self) -> rla::Result<Box<dyn rla::ci::CiPlatform + Send>> {
        Ok(match self {
            CliCiPlatform::Azure => {
                let token = std::env::var("AZURE_DEVOPS_TOKEN")
                    .with_context(|_| "failed to read AZURE_DEVOPS_TOKEN env var")?;
                Box::new(rla::ci::AzurePipelines::new(&token))
            }
            CliCiPlatform::Actions => {
                let token = std::env::var("GITHUB_TOKEN")
                    .with_context(|_| "failed to read GITHUB_TOKEN env var")?;
                Box::new(rla::ci::GitHubActions::new(&token))
            }
        })
    }
}

impl std::str::FromStr for CliCiPlatform {
    type Err = failure::Error;

    fn from_str(input: &str) -> rla::Result<Self> {
        Ok(match input {
            "azure" => CliCiPlatform::Azure,
            "actions" => CliCiPlatform::Actions,
            other => bail!("unknown CI platform: {}", other),
        })
    }
}

pub fn run<F: FnOnce() -> rla::Result<()>>(f: F) {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("RLA_LOG"))
        .init();

    log_and_exit_error(|| f());
}

pub fn log_and_exit_error<F: FnOnce() -> rla::Result<()>>(f: F) {
    if let Err(e) = f() {
        if let Some(v) = e.downcast_ref::<std::io::Error>() {
            if v.kind() == std::io::ErrorKind::BrokenPipe {
                // exit without printing
                process::exit(1);
            }
        }
        error!("{}\n\n{}", e, e.backtrace());
        process::exit(1);
    }
}
