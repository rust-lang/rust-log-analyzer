use super::QueueItem;

use clap;
use regex::bytes::Regex;
use rla;
use std::str;
use std::sync;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use hyper::Uri;

static TRAVIS_TARGET_RUST_PREFIX: &str = "https://travis-ci.com/rust-lang/rust/";

pub struct Worker {
    debug_post: Option<(String, u32)>,
    index_file: PathBuf,
    index: rla::Index,
    extract_config: rla::extract::Config,
    github: rla::github::Client,
    travis: rla::travis::Client,
    queue: sync::mpsc::Receiver<QueueItem>,
}

impl Worker {
    pub fn new(args: &clap::ArgMatches, queue: sync::mpsc::Receiver<QueueItem>) -> rla::Result<Worker> {
        let index_file = Path::new(args.value_of_os("index-file").unwrap());

        let debug_post = match args.value_of("debug-post") {
            None => None,
            Some(v) => {
                let parts = v.splitn(2, '#').collect::<Vec<_>>();
                if parts.len() != 2 {
                    bail!("Invalid debug-post argument: '{}'", v);
                }

                let n = parts[1].parse()?;
                Some((parts[0].to_owned(), n))
            }
        };

        Ok(Worker {
            debug_post,
            index_file: index_file.to_owned(),
            index: rla::Index::load(index_file)?,
            extract_config: Default::default(),
            github: rla::github::Client::new()?,
            travis: rla::travis::Client::new()?,
            queue,
        })
    }

    pub fn main(&mut self) -> rla::Result<()> {
        loop {
            let item = self.queue.recv()?;
            match self.process(item) {
                Ok(()) => (),
                Err(e) => error!("Processing queue item failed: {}", e),
            }
        }
    }

    fn process(&mut self, item: QueueItem) -> rla::Result<()> {
        match item {
            QueueItem::GitHubStatus(ev) => {
                if !(ev.target_url.starts_with(TRAVIS_TARGET_RUST_PREFIX)
                        && ev.context.contains("travis")) {
                    info!("Ignoring non-travis event (ctx: {:?}, url: {:?}).", ev.context, ev.target_url);
                    return Ok(())
                }

                let build_id =
                    Uri::from_str(&ev.target_url)?
                        .path().rsplit('/')
                        .next().ok_or_else(|| format_err!("Invalid event URL."))?
                        .parse()?;

                info!("Processing Travis build #{}...", build_id);

                let build = self.travis.query_build(build_id)?;

                if !build.state.finished() {
                    info!("Ignoring in-progress build.");
                    return Ok(());
                }

                if build.state != rla::travis::JobState::Passed {
                    self.report_failed(&build)?;
                }

                if build.pull_request_number.is_none() && build.branch.name == "auto" {
                    self.learn(&build)?;
                }

                Ok(())
            }
        }
    }

    fn report_failed(&mut self, build: &rla::travis::Build) -> rla::Result<()> {
        debug!("Preparing report...");

        let job = match build.jobs.iter().find(|j| j.state == rla::travis::JobState::Failed || j.state == rla::travis::JobState::Errored) {
            Some(job) => job,
            None => bail!("No failed job found, cannot report."),
        };

        let log = self.travis.query_log(job)?;

        let lines = rla::sanitize::split_lines(&log).iter()
            .map(|l| rla::index::Sanitized(rla::sanitize::clean(l)))
            .collect::<Vec<_>>();

        let blocks = rla::extract::extract(&self.extract_config, &self.index, &lines);

        let blocks = blocks.iter().map(|block|
            block.iter().map(|line| String::from_utf8_lossy(&line.0).into_owned()).collect::<Vec<_>>().join("\n")).collect::<Vec<_>>();

        let extracted = blocks.join("\n---\n");

        let (pr, is_bors) = if let Some(pr) = build.pull_request_number {
            (pr, false)
        } else {
            static BORS_MERGE_PREFIX: &str = "Auto merge of #";

            if build.commit.message.starts_with(BORS_MERGE_PREFIX) {
                let s = &build.commit.message[BORS_MERGE_PREFIX.len()..];
                (s[..s.find(' ').ok_or_else(|| format_err!("Invalid bors commit message: '{}'", build.commit.message))?].parse()?, true)
            } else {
                bail!("Could not determine PR number, cannot report.");
            }
        };

        if !is_bors {
            let pr_info = self.github.query_pr("rust-lang/rust", pr)?;

            let commit_info = self.github.query_commit("rust-lang/rust", &build.commit.sha)?;

            if !commit_info.commit.message.starts_with("Merge ") {
                bail!("Did not recognize commit {} with message '{}', skipping report.", build.commit.sha, commit_info.commit.message);
            }

            let sha = commit_info.commit.message.split(' ').nth(1)
                .ok_or_else(|| format_err!("Did not recognize commit {} with message '{}', skipping report.",
                            build.commit.sha, commit_info.commit.message))?;

            debug!("Extracted head commit sha: '{}'", sha);

            if pr_info.head.sha != sha {
                info!("Build results outdated, skipping report.");
                return Ok(());
            }
        }

        let (repo, pr) = match self.debug_post {
            Some((ref repo, pr_override)) => {
                warn!("Would post to 'rust-lang/rust#{}', debug override to '{}#{}'", pr, repo, pr_override);
                (repo.as_ref(), pr_override)
            }
            None => ("rust-lang/rust", pr),
        };

        let opening = match extract_job_name(&lines) {
            Some(job_name) => format!("The job `{}` of your PR", job_name),
            None => "Your PR".to_owned(),
        };

        self.github.post_comment(repo, pr, &format!(r#"
{opening} [failed on Travis](https://travis-ci.org/rust-lang/rust/jobs/{job}) ([raw log](https://api.travis-ci.org/v3/job/{job}/log.txt)). Through arcane magic we have determined that the following fragments from the build log may contain information about the problem.

<details><summary><i>Click to expand the log.</i></summary>

```plain
{log}
```

</details><p></p>

[I'm a bot](https://github.com/rust-ops/rust-log-analyzer)! I can only do what humans tell me to, so if this was not helpful or you have suggestions for improvements, please ping or otherwise contact **`@TimNN`**. ([Feature Requests](https://github.com/rust-ops/rust-log-analyzer/issues?q=is%3Aopen+is%3Aissue+label%3Afeature-request))
        "#, opening = opening, job = job.id, log = extracted))?;

        Ok(())
    }

    fn learn(&mut self, build: &rla::travis::Build) -> rla::Result<()> {
        for job in &build.jobs {
            if job.state != rla::travis::JobState::Passed {
                continue;
            }

            debug!("Processing travis job {}...", job.id);

            match self.travis.query_log(job) {
                Err(e) => {
                    warn!("Failed to learn from successful travis job {}, download failed: {}",
                          job.id, e);
                    continue;
                }
                Ok(log) => {
                    for line in rla::sanitize::split_lines(&log) {
                        self.index.learn(&rla::index::Sanitized(rla::sanitize::clean(line)), 1);
                    }
                }
            }
        }

        self.index.save(&self.index_file)?;

        Ok(())
    }
}

fn extract_job_name<I: rla::index::IndexData>(lines: &[I]) -> Option<&str> {
    lazy_static! {
        static ref JOB_NAME_PATTERN: Regex = Regex::new("\\[CI_JOB_NAME=([^\\]]+)\\]").unwrap();
    }

    for line in lines {
        if let Some (m) = JOB_NAME_PATTERN.captures(line.sanitized()) {
            return str::from_utf8(m.get(1).unwrap().as_bytes()).ok();
        }
    }

    None
}
