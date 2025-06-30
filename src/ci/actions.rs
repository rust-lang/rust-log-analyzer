use crate::ci::{Build, BuildCommit, CiPlatform, Job, Outcome};
use crate::github::{BuildOutcome, CheckRun};
use crate::Result;
use regex::Regex;
use reqwest::blocking::{Client as ReqwestClient, RequestBuilder, Response};
use reqwest::Method;
use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Deserialize)]
struct ActionsRun {
    id: u64,
    head_branch: String,
    head_sha: String,
    #[serde(flatten)]
    outcome: BuildOutcome,
}

struct GHABuild {
    run: ActionsRun,
    jobs: Vec<GHAJob>,
}

impl GHABuild {
    #[allow(clippy::new_ret_no_self)]
    fn new(client: &Client, repo: &str, run: ActionsRun) -> Result<Box<dyn Build>> {
        let mut jobs = Vec::new();
        client.paginated(
            Method::GET,
            &format!("repos/{}/actions/runs/{}/jobs", repo, run.id),
            &mut |resp| {
                #[derive(Deserialize)]
                struct JobsResult {
                    jobs: Vec<WorkflowJob>,
                }

                let mut partial_jobs: JobsResult = resp.json()?;
                for job in partial_jobs.jobs.drain(..) {
                    jobs.push(GHAJob {
                        inner: job,
                        repo_name: repo.to_string(),
                    });
                }
                Ok(true)
            },
        )?;

        Ok(Box::new(GHABuild { run, jobs }))
    }
}

impl Build for GHABuild {
    fn pr_number(&self) -> Option<u32> {
        // GitHub Actions can't fetch it for us, so let's rely on the detection with log variables
        // (defined in src/log_variables.rs).
        None
    }

    fn branch_name(&self) -> &str {
        &self.run.head_branch
    }

    fn commit_sha(&self) -> BuildCommit<'_> {
        BuildCommit::Head {
            sha: &self.run.head_sha,
        }
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.run.outcome
    }

    fn jobs(&self) -> Vec<&dyn Job> {
        self.jobs.iter().map(|j| j as &dyn Job).collect()
    }
}

#[derive(Deserialize)]
struct WorkflowJob {
    id: usize,
    name: String,
    html_url: String,
    head_sha: String,
    #[serde(flatten)]
    outcome: BuildOutcome,
}

struct GHAJob {
    inner: WorkflowJob,
    repo_name: String,
}

impl Job for GHAJob {
    fn id(&self) -> String {
        self.inner.id.to_string()
    }

    fn html_url(&self) -> String {
        self.inner.html_url.clone()
    }

    fn log_url(&self) -> Option<String> {
        Some(format!(
            "https://github.com/{}/commit/{}/checks/{}/logs",
            self.repo_name, self.inner.head_sha, self.inner.id
        ))
    }

    fn log_api_url(&self) -> Option<String> {
        Some(format!(
            "https://api.github.com/repos/{}/actions/jobs/{}/logs",
            self.repo_name, self.inner.id
        ))
    }

    fn log_enhanced_url(&self) -> Option<String> {
        Some(format!(
            "https://triage.rust-lang.org/gha-logs/{}/{}",
            self.repo_name, self.inner.id
        ))
    }

    fn log_file_name(&self) -> String {
        format!("actions-{}-{}", self.inner.id, self.inner.name)
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.inner.outcome
    }
}

impl std::fmt::Display for GHAJob {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "job {} named {} (outcome={:?})",
            self.inner.id, self.inner.name, self.inner.outcome
        )
    }
}

const GITHUB_ACTIONS_APP_ID: u64 = 15368;

pub struct Client {
    http: ReqwestClient,
    token: String,
}

impl Client {
    pub fn new(token: &str) -> Client {
        Client {
            http: ReqwestClient::new(),
            token: token.to_string(),
        }
    }

    fn req(&self, method: Method, url: &str) -> Result<Response> {
        Ok(self
            .authenticate_request(self.http.request(
                method,
                &if url.starts_with("https://") {
                    url.to_string()
                } else {
                    format!("https://api.github.com/{}", url)
                },
            ))
            .send()?)
    }

    fn paginated(
        &self,
        method: Method,
        url: &str,
        handle: &mut dyn FnMut(Response) -> Result<bool>,
    ) -> Result<()> {
        let mut next_url = Some(url.to_string());
        while let Some(url) = next_url {
            let resp = self.req(method.clone(), &url)?.error_for_status()?;

            // Try to extract the next page URL from the Link header.
            if let Some(Ok(link)) = resp.headers().get("link").map(|l| l.to_str()) {
                next_url = parse_link_header(link)?.remove(&LinkRel::Next);
            } else {
                next_url = None;
            }

            if !handle(resp)? {
                break;
            }
        }
        Ok(())
    }
}

impl CiPlatform for Client {
    fn build_id_from_github_check(&self, e: &crate::github::CheckRunEvent) -> Option<u64> {
        if e.check_run.app.id != GITHUB_ACTIONS_APP_ID {
            return None;
        }

        match fetch_workflow_run_id_from_check_run(self, &e.repository.full_name, &e.check_run) {
            Ok(id) => Some(id),
            Err(err) => {
                debug!("failed to fetch GHA build ID: {}", err);
                None
            }
        }
    }

    fn build_id_from_github_status(&self, _e: &crate::github::CommitStatusEvent) -> Option<u64> {
        None
    }

    fn query_builds(
        &self,
        repo: &str,
        count: u32,
        _offset: u32,
        filter: &dyn Fn(&dyn Build) -> bool,
    ) -> Result<Vec<Box<dyn Build>>> {
        #[derive(Deserialize)]
        struct AllRuns {
            workflow_runs: Vec<ActionsRun>,
        }

        let mut builds = Vec::new();
        self.paginated(
            Method::GET,
            &format!("repos/{}/actions/runs", repo),
            &mut |resp| {
                let mut partial_runs: AllRuns = resp.json()?;
                for run in partial_runs.workflow_runs.drain(..) {
                    if !run.outcome.is_finished() {
                        continue;
                    }

                    let build = GHABuild::new(self, repo, run)?;
                    if filter(build.as_ref()) {
                        builds.push(build);
                    }
                }

                Ok(builds.len() <= count as usize)
            },
        )?;

        Ok(builds)
    }

    fn query_build(&self, repo: &str, id: u64) -> Result<Box<dyn Build>> {
        let run: ActionsRun = self
            .req(Method::GET, &format!("repos/{}/actions/runs/{}", repo, id))?
            .error_for_status()?
            .json()?;
        Ok(GHABuild::new(self, repo, run)?)
    }

    fn remove_timestamp_from_log_line<'a>(&self, line: &'a [u8]) -> Cow<'a, [u8]> {
        // GitHub Actions log lines are always prefixed by the timestamp.
        Cow::Borrowed(line.splitn(2, |c| *c == b' ').last().unwrap_or(line))
    }

    fn authenticate_request(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header(
                reqwest::header::AUTHORIZATION,
                format!("token {}", self.token),
            )
            .header(reqwest::header::USER_AGENT, format!("rust-log-analyzer"))
    }

    fn is_build_outcome_unreliable(&self) -> bool {
        true
    }
}

fn fetch_workflow_run_id_from_check_run(
    client: &Client,
    repo: &str,
    run: &CheckRun,
) -> Result<u64> {
    #[derive(Deserialize)]
    struct ResponseRuns {
        total_count: usize,
        workflow_runs: Vec<ResponseRun>,
    }

    #[derive(Deserialize)]
    struct ResponseRun {
        id: u64,
        check_suite_url: String,
    }

    trace!("starting to fetch workflow run IDs for the {} repo", repo);

    let runs: ResponseRuns = client
        .req(
            Method::GET,
            &format!("repos/{}/actions/runs?per_page=100", repo),
        )?
        .error_for_status()?
        .json()?;

    trace!("received {} workflow runs", runs.total_count);

    for workflow_run in &runs.workflow_runs {
        if workflow_run.check_suite_url == run.check_suite.url {
            trace!("found a matching workflow run");
            return Ok(workflow_run.id);
        }
    }

    anyhow::bail!("can't find the Workflow Run ID from the Check Run");
}

#[derive(Debug, Eq, PartialEq, Hash)]
enum LinkRel {
    First,
    Previous,
    Next,
    Last,
    Other(String),
}

fn parse_link_header(content: &str) -> Result<HashMap<LinkRel, String>> {
    lazy_static! {
        static ref REGEX: Regex = Regex::new(r#"<([^>]+)>; *rel="([^"]+)""#).unwrap();
    }

    let mut result = HashMap::new();
    for entry in content.split(',') {
        if let Some(captures) = REGEX.captures(entry.trim()) {
            let rel = match &captures[2] {
                "first" => LinkRel::First,
                "previous" => LinkRel::Previous,
                "next" => LinkRel::Next,
                "last" => LinkRel::Last,
                other => LinkRel::Other(other.into()),
            };
            result.insert(rel, captures[1].into());
        } else {
            anyhow::bail!("invalid link header entry: {}", entry);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_parse_link_header() {
        let mut expected = HashMap::new();
        expected.insert(LinkRel::Previous, "https://example.com/1".into());
        expected.insert(LinkRel::Next, "https://example.com/3".into());
        expected.insert(
            LinkRel::Other("docs".into()),
            "https://docs.example.com".into(),
        );

        assert_eq!(
            expected,
            parse_link_header(
                "<https://example.com/1>;  rel=\"previous\",
                 <https://example.com/3>; rel=\"next\",
                 <https://docs.example.com>; rel=\"docs\""
            )
            .unwrap(),
        );
    }
}
