use super::{Build, CiPlatform, Job, Outcome};
use crate::Result;
use hyper::{
    header::{self, HeaderValue},
    Uri,
};
use reqwest;
use std::cmp;
use std::env;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

const TRAVIS_API_ID: u64 = 67;
/// The URL parse unescapes the %2F required by Travis, so we use the numeric ID.
const REPO_ID: u64 = 7_321_874;
const TIMEOUT_SECS: u64 = 30;
/// Don't load too many builds at once to avoid timeouts.
const BUILD_PAGE_LIMIT: u32 = 10;
static API_BASE: &str = "https://api.travis-ci.com";

#[derive(Deserialize, Debug)]
struct Repository {
    slug: String,
}

#[derive(Deserialize, Debug)]
struct Builds {
    builds: Vec<TravisBuild>,
}

#[derive(Deserialize, Debug)]
struct TravisBuild {
    branch: Branch,
    commit: Commit,
    pull_request_number: Option<u32>,
    state: JobState,
    jobs: Vec<TravisJob>,
}

impl Build for TravisBuild {
    fn pr_number(&self) -> Option<u32> {
        self.pull_request_number
    }

    fn branch_name(&self) -> &str {
        &self.branch.name
    }

    fn commit_sha(&self) -> &str {
        &self.commit.sha
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.state
    }

    fn jobs(&self) -> Vec<&dyn Job> {
        self.jobs.iter().map(|j| j as &dyn Job).collect()
    }
}

#[derive(Deserialize, Debug)]
struct TravisJob {
    id: u64,
    repository: Repository,
    state: JobState,
}

impl Job for TravisJob {
    fn id(&self) -> String {
        self.id.to_string()
    }

    fn html_url(&self) -> String {
        format!(
            "https://travis-ci.com/{}/jobs/{}",
            self.repository.slug, self.id
        )
    }

    fn log_url(&self) -> Option<String> {
        Some(format!(
            "https://api.travis-ci.com/v3/job/{}/log.txt",
            self.id
        ))
    }

    fn log_file_name(&self) -> String {
        format!("travis-{}-{}", self.id, self.state)
    }

    fn outcome(&self) -> &dyn Outcome {
        &self.state
    }
}

impl fmt::Display for TravisJob {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "travis job {}", self.id)
    }
}

#[derive(Deserialize, Debug)]
struct Branch {
    name: String,
}

#[derive(Deserialize, Debug)]
struct Commit {
    sha: String,
}

#[derive(Copy, Clone, Deserialize, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
enum JobState {
    Received,
    Queued,
    Created,
    Started,
    Passed,
    Canceled,
    Errored,
    Failed,
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::JobState::*;

        let s = match *self {
            Received => "received",
            Queued => "queued",
            Created => "created",
            Started => "started",
            Passed => "passed",
            Canceled => "canceled",
            Errored => "errored",
            Failed => "failed",
        };

        f.pad(s)
    }
}

impl Outcome for JobState {
    fn is_finished(&self) -> bool {
        match *self {
            JobState::Received | JobState::Queued | JobState::Created | JobState::Started => false,
            _ => true,
        }
    }

    fn is_passed(&self) -> bool {
        *self == JobState::Passed
    }

    fn is_failed(&self) -> bool {
        *self == JobState::Failed || *self == JobState::Errored
    }
}

pub struct Client {
    internal: reqwest::Client,
}

impl Client {
    pub fn new() -> Result<Client> {
        let api_key = env::var("TRAVIS_API_KEY")
            .map_err(|e| format_err!("Could not read TRAVIS_API_KEY: {}", e))?;

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("token {}", api_key)).unwrap(),
        );
        headers.insert(
            header::HeaderName::from_static("Travis-API-Version"),
            3.into(),
        );
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static(crate::USER_AGENT),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .referer(false)
            .timeout(Some(Duration::from_secs(TIMEOUT_SECS)))
            .build()?;

        Ok(Client { internal: client })
    }

    fn get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        self.internal
            .get(format!("{}/{}", API_BASE, path).as_str())
            .send()
    }

    fn extract_build_id(&self, url: &str) -> Option<u64> {
        Uri::from_str(url)
            .ok()?
            .path()
            .rsplit('/')
            .next()?
            .parse()
            .ok()
    }
}

impl CiPlatform for Client {
    fn build_id_from_github_check(&self, e: &crate::github::CheckRunEvent) -> Option<u64> {
        if e.check_run.app.id != TRAVIS_API_ID {
            return None;
        }
        self.extract_build_id(&e.check_run.details_url)
    }

    fn build_id_from_github_status(&self, e: &crate::github::CommitStatusEvent) -> Option<u64> {
        if !e.context.starts_with("continuous-integration/travis-ci") {
            return None;
        }
        self.extract_build_id(&e.target_url)
    }

    fn query_builds(
        &self,
        mut count: u32,
        mut offset: u32,
        filter: &dyn Fn(&dyn Build) -> bool,
    ) -> Result<Vec<Box<dyn Build>>> {
        let mut res = vec![];
        while count > 0 {
            let mut resp = self.get(&format!(
                "repo/{}/builds?include=build.jobs&limit={}&offset={}",
                REPO_ID,
                cmp::min(count, BUILD_PAGE_LIMIT),
                offset
            ))?;
            if !resp.status().is_success() {
                bail!("Builds query failed: {:?}", resp);
            }

            for build in resp.json::<Builds>()?.builds.into_iter() {
                offset += 1;
                if filter(&build) {
                    count = count.saturating_sub(1);
                    res.push(Box::new(build) as Box<dyn Build>);
                }
            }
        }

        Ok(res)
    }

    fn query_build(&self, id: u64) -> Result<Box<dyn Build>> {
        let mut resp = self.get(&format!("build/{}?include=build.jobs", id))?;
        if !resp.status().is_success() {
            bail!("Build query failed: {:?}", resp);
        }
        Ok(Box::new(resp.json::<TravisBuild>()?))
    }
}
