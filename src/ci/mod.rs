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
    fn id(&self) -> u64;
    fn html_url(&self) -> String;
    fn log_url(&self) -> String;
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
    fn query_log(&self, job: &dyn Job) -> Result<Vec<u8>>;
}
