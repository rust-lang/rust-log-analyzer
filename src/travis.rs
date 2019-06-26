use super::Result;
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
struct Builds {
    builds: Vec<Build>,
}

#[derive(Deserialize, Debug)]
pub struct Build {
    pub branch: Branch,
    pub commit: Commit,
    pub pull_request_number: Option<u32>,
    pub state: JobState,
    pub jobs: Vec<Job>,
}

#[derive(Deserialize, Debug)]
pub struct Job {
    pub id: u64,
    pub state: JobState,
}

#[derive(Deserialize, Debug)]
pub struct Branch {
    pub name: String,
}

#[derive(Deserialize, Debug)]
pub struct Commit {
    pub message: String,
    pub sha: String,
}

#[derive(Copy, Clone, Deserialize, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
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

impl JobState {
    pub fn finished(self) -> bool {
        match self {
            JobState::Received | JobState::Queued | JobState::Created | JobState::Started => false,
            _ => true,
        }
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
        headers.set(header::UserAgent::new(super::USER_AGENT));

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

    pub fn query_builds(&self, query: &str, mut count: u32, mut offset: u32) -> Result<Vec<Build>> {
        let query = if query.is_empty() {
            "".to_string()
        } else {
            format!("{}&", query)
        };

        let mut res = vec![];

        while count > 0 {
            let mut resp = self.get(&format!(
                "repo/{}/builds?{}include=build.jobs&limit={}&offset={}",
                REPO_ID,
                query,
                cmp::min(count, BUILD_PAGE_LIMIT),
                offset
            ))?;
            if !resp.status().is_success() {
                bail!("Builds query failed: {:?}", resp);
            }

            let builds: Builds = resp.json()?;

            res.extend(builds.builds);

            count = count.saturating_sub(BUILD_PAGE_LIMIT);
            offset += BUILD_PAGE_LIMIT;
        }

        Ok(res)
    }

    pub fn query_build(&self, build_id: u64) -> Result<Build> {
        let mut resp = self.get(&format!("build/{}?include=build.jobs", build_id))?;
        if !resp.status().is_success() {
            bail!("Build query failed: {:?}", resp);
        }

        Ok(resp.json()?)
    }

    pub fn query_log(&self, job: &Job) -> Result<Vec<u8>> {
        let mut resp = self.get(&format!("job/{}/log.txt", job.id))?;

        if !resp.status().is_success() {
            bail!("Downloading log failed: {:?}", resp);
        }

        let mut bytes: Vec<u8> = vec![];
        resp.read_to_end(&mut bytes)?;

        Ok(bytes)
    }
}
