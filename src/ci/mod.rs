use anyhow::anyhow;
use reqwest::blocking::RequestBuilder;
use std::borrow::Cow;
use std::io::Read;

mod actions;
mod azure;

pub use actions::Client as GitHubActions;
pub use azure::Client as AzurePipelines;

use crate::Result;

#[derive(Debug)]
pub enum BuildCommit<'a> {
    Merge { sha: &'a str },
    Head { sha: &'a str },
}

pub trait Outcome: std::fmt::Debug {
    fn is_finished(&self) -> bool;
    fn is_passed(&self) -> bool;
    fn is_failed(&self) -> bool;
}

pub trait Build {
    fn pr_number(&self) -> Option<u32>;
    fn branch_name(&self) -> &str;
    fn commit_sha(&self) -> BuildCommit;
    fn outcome(&self) -> &dyn Outcome;
    fn jobs(&self) -> Vec<&dyn Job>;
}

pub trait Job: std::fmt::Display {
    fn id(&self) -> String;
    fn html_url(&self) -> String;
    fn log_url(&self) -> Option<String>; // sometimes we just don't have log URLs
    fn log_file_name(&self) -> String;
    fn outcome(&self) -> &dyn Outcome;

    fn log_api_url(&self) -> Option<String> {
        self.log_url()
    }

    fn log_enhanced_url(&self) -> Option<String> {
        None
    }
}

pub trait CiPlatform {
    fn build_id_from_github_check(&self, e: &crate::github::CheckRunEvent) -> Option<u64>;
    fn build_id_from_github_status(&self, e: &crate::github::CommitStatusEvent) -> Option<u64>;

    fn query_builds(
        &self,
        repo: &str,
        count: u32,
        offset: u32,
        filter: &dyn Fn(&dyn Build) -> bool,
    ) -> Result<Vec<Box<dyn Build>>>;
    fn query_build(&self, repo: &str, id: u64) -> Result<Box<dyn Build>>;

    fn remove_timestamp_from_log_line<'a>(&self, line: &'a [u8]) -> Cow<'a, [u8]> {
        Cow::Borrowed(line)
    }

    fn authenticate_request(&self, request: RequestBuilder) -> RequestBuilder {
        request
    }

    /// Some CI providers return mismatched data in the API compared to the webhook. Those
    /// providers should return `true` from this method.
    fn is_build_outcome_unreliable(&self) -> bool {
        false
    }
}

pub fn download_log(
    ci: &dyn CiPlatform,
    job: &dyn Job,
    client: &reqwest::blocking::Client,
) -> Option<Result<Vec<u8>>> {
    if let Some(url) = job.log_api_url() {
        let mut resp = match ci.authenticate_request(client.get(&url)).send() {
            Ok(v) => v,
            Err(e) => return Some(Err(e.into())),
        };

        if !resp.status().is_success() {
            return Some(Err(anyhow!("Downloading log failed: {:?}", resp)));
        }

        let mut bytes: Vec<u8> = vec![];
        if let Err(err) = resp.read_to_end(&mut bytes) {
            return Some(Err(err.into()));
        }

        return Some(Ok(bytes));
    }

    None
}
