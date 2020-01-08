use crate::ci::{Build, CiPlatform, Job, Outcome};
use crate::Result;
use regex::Regex;
use reqwest::{Client as ReqwestClient, Method, Response};
use std::collections::HashMap;

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BuildStatus {
    Queued,
    InProgress,
    Completed,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BuildConclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    TimedOut,
    ActionRequired,
}

#[derive(Deserialize, Debug)]
struct BuildOutcome {
    status: BuildStatus,
    conclusion: Option<BuildConclusion>,
}

impl Outcome for BuildOutcome {
    fn is_finished(&self) -> bool {
        self.status == BuildStatus::Completed
    }

    fn is_passed(&self) -> bool {
        self.is_finished() && self.conclusion == Some(BuildConclusion::Success)
    }

    fn is_failed(&self) -> bool {
        self.is_finished() && self.conclusion == Some(BuildConclusion::Failure)
    }
}

#[derive(Deserialize)]
struct CheckSuite {
    head_branch: String,
    head_sha: String,
    #[serde(flatten)]
    outcome: BuildOutcome,
}

struct GHABuild {
    jobs: Vec<GHAJob>,
    check_suite: CheckSuite,
}

impl Build for GHABuild {
    fn pr_number(&self) -> Option<u32> {
        // TODO
        None
    }

    fn branch_name(&self) -> &str {
        &self.check_suite.head_branch
    }

    fn commit_sha(&self) -> &str {
        &self.check_suite.head_sha
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.check_suite.outcome
    }

    fn jobs(&self) -> Vec<&dyn Job> {
        self.jobs.iter().map(|j| j as &dyn Job).collect()
    }
}

#[derive(Deserialize)]
struct CheckRun {
    id: usize,
    name: String,
    html_url: String,
    #[serde(flatten)]
    outcome: BuildOutcome,
}

struct GHAJob {
    repo: String,
    sha: String,
    check_run: CheckRun,
}

impl Job for GHAJob {
    fn id(&self) -> String {
        self.check_run.id.to_string()
    }

    fn html_url(&self) -> String {
        self.check_run.html_url.clone()
    }

    fn log_url(&self) -> Option<String> {
        Some(format!(
            "https://github.com/{}/commit/{}/checks/{}/log",
            self.repo, self.sha, self.check_run.id
        ))
    }

    fn log_file_name(&self) -> String {
        format!("actions-{}-{}", self.check_run.id, self.check_run.name)
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.check_run.outcome
    }
}

impl std::fmt::Display for GHAJob {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "job {} named {} (outcome={:?})",
            self.check_run.id, self.check_run.name, self.check_run.outcome
        )
    }
}

const GITHUB_ACTIONS_APP_ID: u64 = 15368;

pub struct Client {
    http: ReqwestClient,
    repo: String,
    token: String,
}

impl Client {
    pub fn new(repo: &str, token: &str) -> Client {
        Client {
            http: ReqwestClient::new(),
            repo: repo.to_string(),
            token: token.to_string(),
        }
    }

    fn req(&self, method: Method, url: &str) -> Result<Response> {
        Ok(self
            .http
            .request(
                method,
                &if url.starts_with("https://") {
                    url.to_string()
                } else {
                    format!("https://api.github.com/{}", url)
                },
            )
            .header(
                reqwest::header::AUTHORIZATION,
                format!("token {}", self.token),
            )
            .send()?)
    }

    fn paginated(
        &self,
        method: Method,
        url: &str,
        handle: &mut dyn FnMut(Response) -> Result<()>,
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

            handle(resp)?;
        }
        Ok(())
    }
}

impl CiPlatform for Client {
    fn build_id_from_github_check(&self, e: &crate::github::CheckRunEvent) -> Option<u64> {
        if e.check_run.app.id != GITHUB_ACTIONS_APP_ID {
            return None;
        }
        Some(e.check_run.check_suite.id)
    }

    fn build_id_from_github_status(&self, _e: &crate::github::CommitStatusEvent) -> Option<u64> {
        None
    }

    fn query_builds(
        &self,
        _count: u32,
        _offset: u32,
        _filter: &dyn Fn(&dyn Build) -> bool,
    ) -> Result<Vec<Box<dyn Build>>> {
        // There is currently no API to do this, unfortunately.
        unimplemented!();
    }

    fn query_build(&self, id: u64) -> Result<Box<dyn Build>> {
        let check_suite: CheckSuite = self
            .req(
                Method::GET,
                &format!("repos/{}/check-suites/{}", self.repo, id),
            )?
            .error_for_status()?
            .json()?;

        let mut jobs = Vec::new();
        self.paginated(
            Method::GET,
            &format!("repos/{}/check-suites/{}/check-runs", self.repo, id),
            &mut |mut resp| {
                #[derive(Deserialize)]
                struct CheckRunsResult {
                    check_runs: Vec<CheckRun>,
                }

                let mut runs: CheckRunsResult = resp.json()?;
                for check_run in runs.check_runs.drain(..) {
                    jobs.push(GHAJob {
                        repo: self.repo.clone(),
                        sha: check_suite.head_sha.clone(),
                        check_run,
                    });
                }
                Ok(())
            },
        )?;

        Ok(Box::new(GHABuild { jobs, check_suite }))
    }
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
            failure::bail!("invalid link header entry: {}", entry);
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
