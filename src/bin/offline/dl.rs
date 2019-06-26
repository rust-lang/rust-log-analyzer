use crate::offline;
use crate::rla;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;

const LOG_DL_MAX_ATTEMPTS: u32 = 3;

pub fn cat(input: &Path, strip_control: bool, decode_utf8: bool) -> rla::Result<()> {
    let mut data = offline::fs::load_maybe_compressed(input)?;

    if strip_control {
        data.retain(|&b| b == b'\n' || !b.is_ascii_control());
    }

    if decode_utf8 {
        let stdout = io::stdout();
        stdout
            .lock()
            .write_all(String::from_utf8_lossy(&data).as_bytes())?;
    } else {
        let stdout = io::stdout();
        stdout.lock().write_all(&data)?;
    }

    Ok(())
}

pub static TRAVIS_JOB_STATES: &[&str] = &[
    "received", "queued", "created", "started", "passed", "canceled", "errored", "failed",
];

pub static TRAVIS_JOB_STATE_VALUES: &[rla::travis::JobState] = &[
    rla::travis::JobState::Received,
    rla::travis::JobState::Queued,
    rla::travis::JobState::Created,
    rla::travis::JobState::Started,
    rla::travis::JobState::Passed,
    rla::travis::JobState::Canceled,
    rla::travis::JobState::Errored,
    rla::travis::JobState::Failed,
];

pub fn travis(
    output: &Path,
    query: &str,
    count: u32,
    offset: u32,
    job_filter: &[String],
) -> rla::Result<()> {
    let valid_job_states = job_filter
        .iter()
        .map(|f| TRAVIS_JOB_STATE_VALUES[TRAVIS_JOB_STATES.iter().position(|&s| s == f).unwrap()])
        .collect::<HashSet<_>>();

    let travis = rla::travis::Client::new()?;

    let builds = travis.query_builds(query, count, offset)?;

    'job_loop: for job in builds.iter().flat_map(|b| &b.jobs) {
        if !valid_job_states.is_empty() && !valid_job_states.contains(&job.state) {
            continue;
        }

        let save_path = output.join(format!("travis.{}.{}.log.brotli", job.id, job.state));

        if save_path.is_file() {
            warn!(
                "Skipping log for Travis job #{} because the output file exists.",
                job.id
            );
            continue;
        }

        let data;
        let mut attempt = 0;

        loop {
            attempt += 1;

            info!(
                "Downloading log for Travis job #{} [Attempt {}/{}]...",
                job.id, attempt, LOG_DL_MAX_ATTEMPTS
            );

            match travis.query_log(job) {
                Ok(d) => {
                    data = d;
                    break;
                }
                Err(e) => {
                    if attempt >= LOG_DL_MAX_ATTEMPTS {
                        warn!("Failed to download log, skipping: {}", e);
                        continue 'job_loop;
                    }
                }
            }
        }

        debug!("Compressing...");

        offline::fs::save_compressed(&save_path, &data)?;
    }

    Ok(())
}
