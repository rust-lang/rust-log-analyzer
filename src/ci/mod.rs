use std::io::Read;

mod azure;
mod travis;

pub use azure::Client as AzurePipelines;
pub use travis::Client as TravisCI;

use crate::Result;

pub trait Outcome {
    fn is_finished(&self) -> bool;
    fn is_passed(&self) -> bool;
    fn is_failed(&self) -> bool;
}

pub trait Build {
    fn pr_number(&self) -> Option<u32>;
    fn branch_name(&self) -> &str;
    fn commit_sha(&self) -> &str;
    fn outcome(&self) -> &dyn Outcome;
    fn jobs(&self) -> Vec<&dyn Job>;
}

pub trait Job: std::fmt::Display {
    fn id(&self) -> String;
    fn html_url(&self) -> String;
    fn log_url(&self) -> Option<String>; // sometimes we just don't have log URLs
    fn log_file_name(&self) -> String;
    fn outcome(&self) -> &dyn Outcome;
}

pub trait CiPlatform {
    fn build_id_from_github_check(&self, e: &crate::github::CheckRunEvent) -> Option<u64>;
    fn build_id_from_github_status(&self, e: &crate::github::CommitStatusEvent) -> Option<u64>;

    fn query_builds(
        &self,
        count: u32,
        offset: u32,
        filter: &dyn Fn(&dyn Build) -> bool,
    ) -> Result<Vec<Box<dyn Build>>>;
    fn query_build(&self, id: u64) -> Result<Box<dyn Build>>;
}

pub fn download_log(job: &dyn Job, client: &reqwest::Client) -> Option<Result<Vec<u8>>> {
    if let Some(url) = job.log_url() {
        let mut resp = match client.get(&url).send() {
            Ok(v) => v,
            Err(e) => return Some(Err(e.into())),
        };

        if !resp.status().is_success() {
            return Some(Err(failure::err_msg(format!(
                "Downloading log failed: {:?}",
                resp
            ))));
        }

        let mut bytes: Vec<u8> = vec![];
        if let Err(err) = resp.read_to_end(&mut bytes) {
            return Some(Err(err.into()));
        }

        return Some(Ok(bytes));
    }

    None
}
