use crate::ci::{Build, CiPlatform, Job, Outcome};
use crate::Result;
use reqwest::{
    header::{Authorization, Basic},
    Client as ReqwestClient, Method, Response,
};
use std::fmt;

#[derive(Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum BuildResult {
    Canceled,
    Failed,
    None,
    PartiallySucceeded,
    Succeeded,
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
    status: BuildStatus,
}

impl Outcome for BuildOutcome {
    fn is_finished(&self) -> bool {
        self.status == BuildStatus::Completed
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
    log: TimelineLog,
    #[serde(flatten)]
    outcome: BuildOutcome,
}

impl Job for TimelineRecord {
    fn id(&self) -> u64 {
        unreachable!();
    }

    fn html_url(&self) -> String {
        unreachable!();
    }

    fn log_url(&self) -> String {
        self.log.url.clone()
    }

    fn log_file_name(&self) -> String {
        unreachable!();
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.outcome
    }
}

impl fmt::Display for TimelineRecord {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "job TODO of build TODO")
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
    #[serde(flatten)]
    outcome: BuildOutcome,
}

#[derive(Debug)]
struct AzureBuild {
    data: AzureBuildData,
    timeline: Vec<TimelineRecord>,
}

impl AzureBuild {
    fn new(client: &Client, data: AzureBuildData) -> Result<Self> {
        let timeline: Timeline = client
            .req(Method::Get, &data.links.timeline.href)?
            .error_for_status()?
            .json()?;
        Ok(AzureBuild {
            data,
            timeline: timeline.records,
        })
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

    fn commit_sha(&self) -> &str {
        &self.data.source_version
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.data.outcome
    }

    fn jobs(&self) -> Vec<&dyn Job> {
        self.timeline
            .iter()
            .filter(|record| record.type_ == "Job")
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
                &format!("https://dev.azure.com/{}/_apis/{}", self.repo, url),
            )
            .header(Authorization(Basic {
                username: String::new(),
                password: Some(self.token.clone()),
            }))
            .send()?)
    }
}

impl CiPlatform for Client {
    fn build_id_from_github_check(&self, e: &crate::github::CheckRunEvent) -> Option<u64> {
        unimplemented!();
    }

    fn build_id_from_github_status(&self, e: &crate::github::CommitStatusEvent) -> Option<u64> {
        unimplemented!();
    }

    fn query_builds(
        &self,
        count: u32,
        offset: u32,
        filter: &dyn Fn(&dyn Build) -> bool,
    ) -> Result<Vec<Box<dyn Build>>> {
        let builds: AzureBuilds = self
            .req(Method::Get, "build/builds?api-version=5.0")?
            .error_for_status()?
            .json()?;
        for build in builds.value.into_iter() {
            let build = AzureBuild::new(&self, build)?;
            println!("{:?} {}", build.pr_number(), build.branch_name());
        }

        unreachable!();
    }

    fn query_build(&self, id: u64) -> Result<Box<dyn Build>> {
        unimplemented!();
    }

    fn query_log(&self, job: &dyn Job) -> Result<Vec<u8>> {
        unimplemented!();
    }
}
