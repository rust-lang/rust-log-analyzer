use super::{QueueItem, QueueItemKind};

use crate::rla;
use crate::rla::ci::{self, BuildCommit, CiPlatform};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::str;

// We keep track of the last several unique job IDs. This is because
// Azure sends us a notification for every individual builder's
// state (around 70 notifications/job as of this time), but we want
// to only process a given job once.
//
// You might ask -- why is this not a HashSet/HashMap? That would
// also work, but be a little more complicated to remove things
// from. We would need to keep track of order somehow to remove the
// oldest job ID. An attempt at such an API was tried in PR #29, but
// ultimately scrapped as too complicated.
//
// We keep few enough elements in this "set" that a Vec isn't too bad.
//
// Note: Don't update this number too high, as we O(n) loop through it on every
// notification from GitHub (twice).
const KEEP_IDS: usize = 16;

pub struct Worker {
    debug_post: Option<(String, u32)>,
    index_file: PathBuf,
    index: rla::Index,
    extract_config: rla::extract::Config,
    github: rla::github::Client,
    queue: crossbeam::channel::Receiver<QueueItem>,
    notified: VecDeque<u64>,
    ci: Box<dyn CiPlatform + Send>,
    repo: String,
    secondary_repos: Vec<String>,
    query_builds_from_primary_repo: bool,
}

impl Worker {
    pub fn new(
        index_file: PathBuf,
        debug_post: Option<String>,
        queue: crossbeam::channel::Receiver<QueueItem>,
        ci: Box<dyn CiPlatform + Send>,
        repo: String,
        secondary_repos: Vec<String>,
        query_builds_from_primary_repo: bool,
    ) -> rla::Result<Worker> {
        let debug_post = match debug_post {
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
            index: rla::Index::load(&index_file)?,
            index_file,
            extract_config: Default::default(),
            github: rla::github::Client::new()?,
            notified: VecDeque::new(),
            queue,
            ci,
            repo,
            secondary_repos,
            query_builds_from_primary_repo,
        })
    }

    pub fn main(&mut self) -> rla::Result<()> {
        loop {
            let item = self.queue.recv()?;

            let span = span!(
                tracing::Level::INFO,
                "request",
                delivery = item.delivery_id.as_str(),
                build_id = tracing::field::Empty
            );
            let _enter = span.enter();

            match self.process(item, &span) {
                Ok(()) => (),
                Err(e) => error!("Processing queue item failed: {}", e),
            }
        }
    }

    fn is_repo_valid(&self, repo: &str) -> bool {
        if repo == self.repo {
            return true;
        }
        self.secondary_repos.iter().find(|r| *r == repo).is_some()
    }

    fn process(&mut self, item: QueueItem, span: &tracing::Span) -> rla::Result<()> {
        let (repo, build_id, outcome) = match &item.kind {
            QueueItemKind::GitHubStatus(ev) => match self.ci.build_id_from_github_status(&ev) {
                Some(id) if self.is_repo_valid(&ev.repository.full_name) => {
                    (&ev.repository.full_name, id, None)
                }
                _ => {
                    info!(
                        "Ignoring invalid event (ctx: {:?}, url: {:?}).",
                        ev.context, ev.target_url
                    );
                    return Ok(());
                }
            },
            QueueItemKind::GitHubCheckRun(ev) => match self.ci.build_id_from_github_check(&ev) {
                Some(id) if self.is_repo_valid(&ev.repository.full_name) => {
                    (&ev.repository.full_name, id, Some(&ev.check_run.outcome))
                }
                _ => {
                    info!(
                        "Ignoring invalid event (app id: {:?}, url: {:?}).",
                        ev.check_run.app.id, ev.check_run.details_url
                    );
                    return Ok(());
                }
            },
        };

        span.record("build_id", &build_id);

        info!("started processing");

        if self.notified.contains(&build_id) {
            info!("ignoring recently notified build");
            return Ok(());
        }
        let query_from = if self.query_builds_from_primary_repo {
            &self.repo
        } else {
            repo
        };
        let build = self.ci.query_build(query_from, build_id)?;

        let outcome = match outcome {
            Some(outcome) if self.ci.is_build_outcome_unreliable() => &*outcome,
            _ => build.outcome(),
        };

        debug!("current outcome: {:?}", outcome);
        debug!("PR number: {:?}", build.pr_number());
        debug!("branch name: {:?}", build.branch_name());

        if !outcome.is_finished() {
            info!("ignoring in-progress build");
            return Ok(());
        }

        // Avoid processing the same build multiple times.
        if !outcome.is_passed() {
            info!("preparing report");
            self.report_failed(build.as_ref())?;

            info!("marked as notified");
            self.notified.push_front(build_id);
            if self.notified.len() > KEEP_IDS {
                self.notified.pop_back();
            }
        }
        if build.pr_number().is_none() && build.branch_name() == "auto" {
            info!("learning from the log");
            self.learn(build.as_ref())?;
        } else {
            info!("did not learn as it's not an auto build");
        }

        Ok(())
    }

    fn report_failed(&mut self, build: &dyn rla::ci::Build) -> rla::Result<()> {
        debug!("Preparing report...");

        let job = match build.jobs().iter().find(|j| j.outcome().is_failed()) {
            Some(job) => *job,
            None => bail!("No failed job found, cannot report."),
        };

        let log = match ci::download_log(self.ci.as_ref(), job, self.github.internal()) {
            Some(res) => res?,
            None => bail!("No log for failed job"),
        };

        let lines = rla::sanitize::split_lines(&log)
            .iter()
            .map(|l| rla::index::Sanitized(rla::sanitize::clean(self.ci.as_ref(), l)))
            .collect::<Vec<_>>();

        let blocks = rla::extract::extract(&self.extract_config, &self.index, &lines);

        let blocks = blocks
            .iter()
            .map(|block| {
                block
                    .iter()
                    .map(|line| String::from_utf8_lossy(&line.0).into_owned())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>();

        let extracted = blocks.join("\n---\n");

        // Some CI providers return a merge commit instead of the head commit of the branch/PR when
        // querying the build. If the provider returned a merge commit, this fetches the related
        // head commit from the GitHub API.
        let commit_sha = match build.commit_sha() {
            BuildCommit::Head { sha } => sha.to_string(),
            BuildCommit::Merge { sha } => {
                let mut commit = self.github.query_commit(&self.repo, sha)?;
                if commit.parents.len() > 1 {
                    // The first parent is master, the second parent is the branch/PR.
                    commit.parents.remove(1).sha
                } else {
                    bail!("commit {} is not a merge commit", sha);
                }
            }
        };

        let commit_message = self
            .github
            .query_commit(&self.repo, &commit_sha)?
            .commit
            .message;

        let log_variables = rla::log_variables::LogVariables::extract(&lines);

        let (pr, is_bors) = if let Some(pr) = build.pr_number() {
            (pr, false)
        } else {
            static BORS_MERGE_PREFIX: &str = "Auto merge of #";

            if commit_message.starts_with(BORS_MERGE_PREFIX) {
                let s = &commit_message[BORS_MERGE_PREFIX.len()..];
                (
                    s[..s.find(' ').ok_or_else(|| {
                        format_err!("Invalid bors commit message: '{}'", commit_message)
                    })?]
                        .parse()?,
                    true,
                )
            } else if let Some(number) = log_variables.pr_number {
                (number.parse()?, false)
            } else {
                bail!("Could not determine PR number, cannot report.");
            }
        };

        if !is_bors {
            let pr_info = self.github.query_pr(&self.repo, pr)?;
            if pr_info.head.sha != commit_sha {
                info!("Build results outdated, skipping report.");
                return Ok(());
            }
        }

        let (repo, pr) = match self.debug_post {
            Some((ref repo, pr_override)) => {
                warn!(
                    "Would post to '{}#{}', debug override to '{}#{}'",
                    self.repo, pr, repo, pr_override
                );
                (repo.as_str(), pr_override)
            }
            None => (self.repo.as_str(), pr),
        };

        let opening = match log_variables.job_name {
            Some(job_name) => format!("The job `{}` of your PR", job_name),
            None => "Your PR".to_owned(),
        };

        let log_url = job.log_url().unwrap_or_else(|| "unknown".into());
        let pretty_log_url = format!(
            "https://rust-lang.github.io/rust-log-analyzer/log-viewer/#{}",
            &log_url
        );
        let raw_log_url = log_url;
        self.github.post_comment(repo, pr, &format!(r#"
{opening} [failed]({html_url}) ([pretty log]({log_url}), [raw log]({raw_log_url})). Through arcane magic we have determined that the following fragments from the build log may contain information about the problem.

<details><summary><i>Click to expand the log.</i></summary>

```plain
{log}
```

</details><p></p>

[I'm a bot](https://github.com/rust-lang/rust-log-analyzer)! I can only do what humans tell me to, so if this was not helpful or you have suggestions for improvements, please ping or otherwise contact **`@rust-lang/infra`**. ([Feature Requests](https://github.com/rust-lang/rust-log-analyzer/issues?q=is%3Aopen+is%3Aissue+label%3Afeature-request))
        "#, opening = opening, html_url = job.html_url(), log_url = pretty_log_url, raw_log_url = raw_log_url, log = extracted))?;

        Ok(())
    }

    fn learn(&mut self, build: &dyn rla::ci::Build) -> rla::Result<()> {
        for job in &build.jobs() {
            if !job.outcome().is_passed() {
                continue;
            }

            debug!("Processing {}...", job);

            match ci::download_log(self.ci.as_ref(), *job, self.github.internal()) {
                Some(Ok(log)) => {
                    for line in rla::sanitize::split_lines(&log) {
                        self.index.learn(
                            &rla::index::Sanitized(rla::sanitize::clean(self.ci.as_ref(), line)),
                            1,
                        );
                    }
                }
                None => {
                    warn!(
                        "Failed to learn from successful {}, download failed; no log",
                        job
                    );
                }
                Some(Err(e)) => {
                    warn!(
                        "Failed to learn from successful {}, download failed: {}",
                        job, e
                    );
                }
            }
        }

        self.index.save(&self.index_file)?;

        Ok(())
    }
}
