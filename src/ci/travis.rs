use super::{Build, CiPlatform, Job, Outcome};
use crate::Result;
use hyper::header;
use reqwest;
use std::cmp;
use std::env;
use std::fmt;
use std::io::Read;
use std::str;
use std::time::Duration;

header! { (XTravisApiVersion, "Travis-API-Version") => [u8] }

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

    fn commit_message(&self) -> &str {
        &self.commit.message
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
    fn id(&self) -> u64 {
        self.id
    }

    fn html_url(&self) -> String {
        format!(
            "https://travis-ci.com/{}/jobs/{}",
            self.repository.slug, self.id
        )
    }

    fn log_url(&self) -> String {
        format!("https://api.travis-ci.com/v3/job/{}/log.txt", self.id)
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
    message: String,
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

        let mut headers = header::Headers::new();
        headers.set(header::Authorization(format!("token {}", api_key)));
        headers.set(XTravisApiVersion(3));
        headers.set(header::UserAgent::new(crate::USER_AGENT));

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
}

impl CiPlatform for Client {
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

    fn query_build(&self, id: u64) -> Result<Box<Build>> {
        let mut resp = self.get(&format!("build/{}?include=build.jobs", id))?;
        if !resp.status().is_success() {
            bail!("Build query failed: {:?}", resp);
        }
        Ok(Box::new(resp.json::<TravisBuild>()?))
    }

    fn query_log(&self, job: &dyn Job) -> Result<Vec<u8>> {
        let mut resp = self.get(&format!("job/{}/log.txt", job.id()))?;

        if !resp.status().is_success() {
            bail!("Downloading log failed: {:?}", resp);
        }

        let mut bytes: Vec<u8> = vec![];
        resp.read_to_end(&mut bytes)?;

        Ok(bytes)
    }
}
