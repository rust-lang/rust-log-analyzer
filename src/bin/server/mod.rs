use crate::rla;

pub use self::service::RlaService;
pub use self::worker::Worker;

mod service;
mod worker;

pub struct QueueItem {
    pub kind: QueueItemKind,
    pub delivery_id: String,
}

pub enum QueueItemKind {
    GitHubStatus(rla::github::CommitStatusEvent),
    GitHubCheckRun(rla::github::CheckRunEvent),
    GitHubPullRequest(rla::github::PullRequestEvent),
}
