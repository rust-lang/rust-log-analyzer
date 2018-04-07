use clap;
use offline;
use rla;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

const LOG_DL_MAX_ATTEMPTS: u32 = 3;

pub fn cat(args: &clap::ArgMatches) -> rla::Result<()> {
    let input = Path::new(args.value_of_os("input").unwrap());

    let mut data = offline::fs::load(input)?;

    if args.is_present("strip-control") {
        data.retain(|&b| b == b'\n' || !b.is_ascii_control());
    }

    if args.is_present("decode-utf8") {
        let stdout = io::stdout();
        stdout.lock().write_all(String::from_utf8_lossy(&data).as_bytes())?;
    } else {
        let stdout = io::stdout();
        stdout.lock().write_all(&data)?;
    }

    Ok(())
}

pub fn travis(args: &clap::ArgMatches) -> rla::Result<()> {
    let count: u32 = args.value_of("count").unwrap().parse()?;
    let offset: u32 = args.value_of("skip").unwrap_or("0").parse()?;
    let output = Path::new(args.value_of_os("output").unwrap());
    let query = args.value_of("query").unwrap();
    let valid_job_states = args.value_of("job-filter")
        .map(|v|
            v.split(',')
                .map(rla::travis::JobState::from_str)
                .collect::<rla::Result<HashSet<_>>>())
        .unwrap_or_else(|| Ok(HashSet::new()))?;

    let travis = rla::travis::Client::new()?;

    let builds = travis.query_builds(query, count, offset)?;

    'job_loop:
    for job in builds.iter().flat_map(|b| &b.jobs) {
        if !valid_job_states.is_empty() && !valid_job_states.contains(&job.state) {
            continue;
        }

        let data;
        let mut attempt = 0;

        loop {
            attempt += 1;

            info!("Downloading log for Travis job #{} [Attempt {}/{}]...",
                  job.id, attempt, LOG_DL_MAX_ATTEMPTS);

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

        offline::fs::save(&output.join(format!("travis.{}.{}.log.brotli", job.id, job.state)),
                          &data)?;
    }

    Ok(())
}
