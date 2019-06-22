use crate::offline;
use crate::rla;
use crate::rla::ci::CiPlatform;
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

pub fn travis(
    output: &Path,
    count: u32,
    offset: u32,
    filter_branches: &[String],
    only_passed: bool,
    only_failed: bool,
) -> rla::Result<()> {
    let filter_branches = filter_branches
        .iter()
        .map(|s| s.as_str())
        .collect::<HashSet<_>>();
    let travis = rla::ci::TravisCI::new()?;

    let check_outcome = |outcome: &dyn rla::ci::Outcome| {
        (!only_passed || outcome.is_passed()) && (!only_failed || outcome.is_failed())
    };
    let builds = travis.query_builds(count, offset, &|build| {
        (filter_branches.is_empty() || filter_branches.contains(build.branch_name()))
            && check_outcome(build.outcome())
    })?;

    'job_loop: for job in builds.iter().flat_map(|b| b.jobs()) {
        if !check_outcome(job.outcome()) {
            continue;
        }

        let save_path = output.join(format!("travis.{}.{}.log.brotli", job.id(), job.outcome()));

        if save_path.is_file() {
            warn!("Skipping log for {} because the output file exists.", job);
            continue;
        }

        let data;
        let mut attempt = 0;

        loop {
            attempt += 1;
            info!(
                "Downloading log for {} [Attempt {}/{}]...",
                job, attempt, LOG_DL_MAX_ATTEMPTS
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
