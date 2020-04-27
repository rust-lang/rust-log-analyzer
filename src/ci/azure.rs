#![allow(unused)]
use crate::ci::{Build, BuildCommit, CiPlatform, Job, Outcome};
use crate::Result;
use failure::ResultExt;
use reqwest::{Client as ReqwestClient, Method, Response, StatusCode};
use std::fmt;
use std::io::Read;
use std::borrow::Cow;

#[derive(Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum BuildResult {
    Canceled,
    Failed,
    None,
    PartiallySucceeded,
    Succeeded,
    Skipped,
    Abandoned,
    SucceededWithIssues,
}

#[derive(Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum BuildStatus {
    All,
    Cancelling,
    Completed,
    InProgress,
    None,
    NotStarted,
    Postponed,
}

#[derive(Debug, Deserialize)]
struct BuildOutcome {
    result: Option<BuildResult>,
    status: Option<BuildStatus>,
}

impl Outcome for BuildOutcome {
    fn is_finished(&self) -> bool {
        // TimelineRecord of type Job does not have a status
        self.status == Some(BuildStatus::Completed) || self.status.is_none()
    }

    fn is_passed(&self) -> bool {
        self.is_finished() && self.result == Some(BuildResult::Succeeded)
    }

    fn is_failed(&self) -> bool {
        self.is_finished() && self.result == Some(BuildResult::Failed)
    }
}

#[derive(Debug, Deserialize)]
struct TimelineLog {
    url: String,
}

#[derive(Debug, Deserialize)]
struct TimelineRecord {
    #[serde(rename = "type")]
    type_: String,
    id: String,
    name: String,
    log: Option<TimelineLog>,
    #[serde(flatten)]
    outcome: BuildOutcome,
}

#[derive(Debug, Deserialize)]
struct TaskReference {
    id: String,
    name: String,
    version: String,
}

impl TimelineRecord {
    fn log(&self) -> &TimelineLog {
        self.log.as_ref().unwrap_or_else(|| {
            panic!("log field = None for {:?}", self);
        })
    }
}

#[derive(Debug)]
struct AzureJob {
    record: TimelineRecord,
    build: u64,
    repo: String,
}

impl Job for AzureJob {
    fn id(&self) -> String {
        self.record.id.clone()
    }

    fn html_url(&self) -> String {
        format!(
            "https://dev.azure.com/{repo}/_build/results?buildId={build}&view=logs&jobId={job}",
            repo = self.repo, build = self.build, job = self.record.id
        )
    }

    fn log_url(&self) -> Option<String> {
        self.record.log.as_ref().map(|l| l.url.clone())
    }

    fn log_file_name(&self) -> String {
        format!("azure-{}-{}", self.id(), self.build)
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.record.outcome
    }
}

impl fmt::Display for AzureJob {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "job {} of build {} (outcome={:?})",
            self.record.name, self.build, self.record.outcome
        )
    }
}

#[derive(Debug, Deserialize)]
struct Timeline {
    records: Vec<TimelineRecord>,
}

#[derive(Debug, Deserialize)]
struct TriggerInfo {
    #[serde(rename = "pr.number")]
    pr_number: Option<String>,
    #[serde(rename = "pr.sourceBranch")]
    pr_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Link {
    href: String,
}

#[derive(Debug, Deserialize)]
struct AzureBuildLinks {
    timeline: Link,
    #[allow(unused)]
    web: Link,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AzureBuildData {
    #[serde(rename = "_links")]
    links: AzureBuildLinks,
    id: u64,
    trigger_info: TriggerInfo,
    source_branch: String,
    source_version: String,
    build_number: String,
    #[serde(flatten)]
    outcome: BuildOutcome,
}

#[derive(Debug)]
struct AzureBuild {
    data: AzureBuildData,
    jobs: Vec<AzureJob>,
}

impl AzureBuild {
    fn new(client: &Client, repo: &str, data: AzureBuildData) -> Result<Option<Self>> {
        let mut resp = client
            .req(Method::GET, repo, &data.links.timeline.href)?
            .error_for_status()?;
        // this means that the build didn't parse from the yaml,
        // or at least that's the one case we've hit so far
        if resp.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let timeline: Timeline = resp.json().with_context(|_| format!("{:?}", resp))?;
        Ok(Some(AzureBuild {
            jobs: timeline
                .records
                .into_iter()
                .map(|record| AzureJob {
                    build: data.id,
                    repo: repo.to_string(),
                    record,
                })
                .collect(),
            data,
        }))
    }
}

impl Build for AzureBuild {
    fn pr_number(&self) -> Option<u32> {
        self.data
            .trigger_info
            .pr_number
            .as_ref()
            .and_then(|num| num.parse().ok())
    }

    fn branch_name(&self) -> &str {
        const HEAD_PREFIX: &str = "refs/heads/";
        if let Some(branch) = &self.data.trigger_info.pr_branch {
            &branch
        } else if self.data.source_branch.starts_with(HEAD_PREFIX) {
            &self.data.source_branch[HEAD_PREFIX.len()..]
        } else {
            &self.data.source_branch
        }
    }

    fn commit_sha(&self) -> BuildCommit<'_> {
        // Azure Pipelines returns merge commits for PRs and head commits for branches
        if self.data.trigger_info.pr_number.is_some() {
            BuildCommit::Merge {
                sha: &self.data.source_version,
            }
        } else {
            BuildCommit::Head {
                sha: &self.data.source_version,
            }
        }
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.data.outcome
    }

    fn jobs(&self) -> Vec<&dyn Job> {
        self.jobs
            .iter()
            .filter(|job| job.record.type_ == "Job")
            // Azure does not properly publish logs for canceled builds. These builds are the ones
            // that cancelbot killed manually, vs. the "failed" builds, so we don't care too much
            // about them for now, and just ignore them here
            .filter(|job| job.record.outcome.result != Some(BuildResult::Canceled))
            .map(|job| job as &dyn Job)
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct AzureBuilds {
    count: usize,
    value: Vec<AzureBuildData>,
}

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

    fn req(&self, method: Method, repo: &str, url: &str) -> Result<Response> {
        Ok(self
            .http
            .request(
                method,
                &if url.starts_with("https://") {
                    url.to_owned()
                } else {
                    format!("https://dev.azure.com/{}/_apis/{}", repo, url)
                },
            )
            .basic_auth("", Some(self.token.clone()))
            .send()?)
    }
}

const AZURE_API_ID: u64 = 9426;

impl CiPlatform for Client {
    fn build_id_from_github_check(&self, e: &crate::github::CheckRunEvent) -> Option<u64> {
        if e.check_run.app.id != AZURE_API_ID {
            return None;
        }
        e.check_run
            .external_id
            .split('|')
            .nth(1)
            .and_then(|id| id.parse().ok())
    }

    fn build_id_from_github_status(&self, e: &crate::github::CommitStatusEvent) -> Option<u64> {
        None
    }

    fn query_builds(
        &self,
        repo: &str,
        count: u32,
        offset: u32,
        filter: &dyn Fn(&dyn Build) -> bool,
    ) -> Result<Vec<Box<dyn Build>>> {
        let resp = self.req(
            Method::GET,
            repo,
            &format!("build/builds?api-version=5.0&$top={}", count),
        )?;
        let mut resp = resp.error_for_status()?;
        let builds: AzureBuilds = resp.json()?;
        let mut ret = Vec::new();
        for build in builds.value.into_iter() {
            if build.outcome.status == Some(BuildStatus::InProgress) {
                continue;
            }
            if let Some(build) = AzureBuild::new(&self, repo, build)? {
                println!(
                    "id={} pr={:?} branch_name={}, commit={}, status={:?}",
                    build.data.id,
                    build.pr_number(),
                    build.branch_name(),
                    build.data.source_version,
                    build.data.outcome,
                );
                if filter(&build) {
                    ret.push(Box::new(build) as Box<dyn Build>);
                }
            }
        }

        eprintln!("pushed {:?}", ret.len());

        Ok(ret)
    }

    fn query_build(&self, repo: &str, id: u64) -> Result<Box<dyn Build>> {
        let resp = self.req(Method::GET, repo, &format!("build/builds/{}?api-version=5.0", id))?;
        let mut resp = resp.error_for_status()?;
        let data: AzureBuildData = resp.json()?;
        if let Some(build) = AzureBuild::new(&self, repo, data)? {
            Ok(Box::new(build))
        } else {
            Err(failure::err_msg("no build results"))
        }
    }

    fn remove_timestamp_from_log_line<'a>(&self, line: &'a [u8]) -> Cow<'a, [u8]> {
        // Azure Pipelines log lines are always prefixed by the timestamp.
        Cow::Borrowed(line.splitn(2, |c| *c == b' ').last().unwrap_or(line))
    }
}
