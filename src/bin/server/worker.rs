use super::QueueItem;

use crate::rla;
use crate::rla::ci::{self, BuildCommit, CiPlatform};
use anyhow::bail;
use rla::index::IndexStorage;
use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::str;
use std::time::{Duration, Instant};

const MINIMUM_DELAY_BETWEEN_INDEX_BACKUPS: Duration = Duration::from_secs(60 * 60);
const SILENCE_LABEL: &str = "rla-silenced";

pub struct Worker {
    debug_post: Option<(String, u32)>,
    index_file: IndexStorage,
    index: rla::Index,
    extract_config: rla::extract::Config,
    github: rla::github::Client,
    queue: crossbeam::channel::Receiver<QueueItem>,
    ci: Box<dyn CiPlatform + Send>,
    repo: String,
    secondary_repos: Vec<String>,
    query_builds_from_primary_repo: bool,

    recently_notified: RecentlySeen<u64>,
    recently_learned: RecentlySeen<String>,

    last_index_backup: Option<Instant>,
}

impl Worker {
    pub fn new(
        index_file: IndexStorage,
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
            queue,
            ci,
            repo,
            secondary_repos,
            query_builds_from_primary_repo,

            recently_notified: RecentlySeen::new(32),
            recently_learned: RecentlySeen::new(256),

            last_index_backup: None,
        })
    }

    pub fn main(&mut self) -> rla::Result<()> {
        loop {
            let item = self.queue.recv()?;

            let span = span!(
                tracing::Level::INFO,
                "request",
                delivery = item.delivery_id(),
                build_id = tracing::field::Empty
            );
            let _enter = span.enter();

            match self.process(item, &span) {
                Ok(ProcessOutcome::Continue) => (),
                Ok(ProcessOutcome::Exit) => return Ok(()),
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

    fn process(&mut self, item: QueueItem, span: &tracing::Span) -> rla::Result<ProcessOutcome> {
        let (repo, build_id, outcome) = match &item {
            QueueItem::GitHubStatus { payload, .. } => {
                match self.ci.build_id_from_github_status(&payload) {
                    Some(id) if self.is_repo_valid(&payload.repository.full_name) => {
                        (&payload.repository.full_name, id, None)
                    }
                    _ => {
                        info!(
                            "Ignoring invalid event (ctx: {:?}, url: {:?}).",
                            payload.context, payload.target_url
                        );
                        return Ok(ProcessOutcome::Continue);
                    }
                }
            }
            QueueItem::GitHubCheckRun { payload, .. } => {
                match self.ci.build_id_from_github_check(&payload) {
                    Some(id) if self.is_repo_valid(&payload.repository.full_name) => (
                        &payload.repository.full_name,
                        id,
                        Some(&payload.check_run.outcome),
                    ),
                    _ => {
                        info!(
                            "Ignoring invalid event (app id: {:?}, url: {:?}).",
                            payload.check_run.app.id, payload.check_run.details_url
                        );
                        return Ok(ProcessOutcome::Continue);
                    }
                }
            }
            QueueItem::GitHubPullRequest { payload, .. } => {
                self.process_pr(payload)?;
                return Ok(ProcessOutcome::Continue);
            }

            QueueItem::GracefulShutdown => {
                info!("persisting the index to disk before shutting down");
                self.index.save(&self.index_file)?;
                return Ok(ProcessOutcome::Exit);
            }
        };

        span.record("build_id", &build_id);

        info!("started processing");

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
            return Ok(ProcessOutcome::Continue);
        }

        // Avoid processing the same build multiple times.
        if !outcome.is_passed() {
            self.report_failed(build_id, build.as_ref())?;
        }
        if build.pr_number().is_none() && build.branch_name() == "auto" {
            info!("learning from the log");
            self.learn(build.as_ref())?;
        } else {
            info!("did not learn as it's not an auto build");
        }

        Ok(ProcessOutcome::Continue)
    }

    fn report_failed(&mut self, build_id: u64, build: &dyn rla::ci::Build) -> rla::Result<()> {
        if self.recently_notified.recently_witnessed(&build_id) {
            info!("avoided reporting recently notified build");
            return Ok(());
        }

        info!("preparing report");

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
                        anyhow::format_err!("Invalid bors commit message: '{}'", commit_message)
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
            if pr_info
                .labels
                .iter()
                .any(|label| label.name == SILENCE_LABEL)
            {
                info!("PR has label `{SILENCE_LABEL}`, skipping report");
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
            Some(job_name) => format!("The job **`{}`**", job_name),
            None => "A job".to_owned(),
        };

        let log_url = job.log_url().unwrap_or_else(|| "unknown".into());
        self.github.post_comment(repo, pr, &format!(r#"
{opening} failed! Check out the build log: [(web)]({html_url}) [(plain)]({log_url})

<details><summary><i>Click to see the possible cause of the failure (guessed by this bot)</i></summary>

```plain
{log}
```

</details>
        "#, opening = opening, html_url = job.html_url(), log_url = log_url, log = extracted))?;

        info!("marked build {} as recently notified", build_id);
        self.recently_notified.store(build_id);

        Ok(())
    }

    fn learn(&mut self, build: &dyn rla::ci::Build) -> rla::Result<()> {
        for job in &build.jobs() {
            if !job.outcome().is_passed() {
                continue;
            }

            if self.recently_learned.recently_witnessed(&job.id()) {
                trace!("Skipped already processed {}", job);
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
                    self.recently_learned.store(job.id());
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

        // To avoid persisting the index too many times to storage, we only persist it after some
        // time elapsed since the last save.
        match self.last_index_backup {
            Some(last) if last.elapsed() >= MINIMUM_DELAY_BETWEEN_INDEX_BACKUPS => {
                self.last_index_backup = Some(Instant::now());
                self.index.save(&self.index_file)?;
            }
            Some(_) => {}
            None => self.last_index_backup = Some(Instant::now()),
        }

        Ok(())
    }

    fn process_pr(&self, e: &rla::github::PullRequestEvent) -> rla::Result<()> {
        // Hide all comments by the bot when a new commit is pushed.
        if let rla::github::PullRequestAction::Synchronize = e.action {
            self.github
                .hide_own_comments(&e.repository.full_name, e.number)?;
        }
        Ok(())
    }
}

/// Keeps track of the recently seen IDs for both the failed build reports and the learned jobs.
/// Only the most recent IDs are stored, to avoid growing the memory usage endlessly.
///
/// Internally this uses both an HashSet to provide fast lookups and a VecDeque to know which old
/// jobs needs to be removed.
struct RecentlySeen<T: Clone + Eq + Hash> {
    size: usize,
    lookup: HashSet<T>,
    removal: VecDeque<T>,
}

impl<T: Clone + Eq + Hash> RecentlySeen<T> {
    fn new(size: usize) -> Self {
        Self {
            size,
            lookup: HashSet::with_capacity(size),
            removal: VecDeque::with_capacity(size),
        }
    }

    fn recently_witnessed(&self, key: &T) -> bool {
        self.lookup.contains(key)
    }

    fn store(&mut self, key: T) {
        if self.lookup.contains(&key) {
            return;
        }
        if self.removal.len() >= self.size {
            if let Some(item) = self.removal.pop_back() {
                self.lookup.remove(&item);
            }
        }
        self.lookup.insert(key.clone());
        self.removal.push_front(key);
    }
}

enum ProcessOutcome {
    Continue,
    Exit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recently_seen() {
        let mut recently = RecentlySeen::new(2);

        assert!(!recently.recently_witnessed(&0));
        assert!(!recently.recently_witnessed(&1));
        assert!(!recently.recently_witnessed(&2));

        recently.store(0);
        assert!(recently.recently_witnessed(&0));
        assert!(!recently.recently_witnessed(&1));
        assert!(!recently.recently_witnessed(&2));

        recently.store(1);
        assert!(recently.recently_witnessed(&0));
        assert!(recently.recently_witnessed(&1));
        assert!(!recently.recently_witnessed(&2));

        recently.store(2);
        assert!(!recently.recently_witnessed(&0));
        assert!(recently.recently_witnessed(&1));
        assert!(recently.recently_witnessed(&2));
    }
}
