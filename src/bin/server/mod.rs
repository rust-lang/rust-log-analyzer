use crate::rla;

pub use self::service::RlaService;
pub use self::worker::Worker;

mod service;
mod worker;

pub enum QueueItem {
    GitHubStatus {
        payload: rla::github::CommitStatusEvent,
        delivery_id: String,
    },
    GitHubCheckRun {
        payload: rla::github::CheckRunEvent,
        delivery_id: String,
    },
    GitHubPullRequest {
        payload: rla::github::PullRequestEvent,
        delivery_id: String,
    },
    GracefulShutdown,
}

impl QueueItem {
    fn delivery_id(&self) -> Option<&str> {
        match self {
            QueueItem::GitHubStatus { delivery_id, .. } => Some(&delivery_id),
            QueueItem::GitHubCheckRun { delivery_id, .. } => Some(&delivery_id),
            QueueItem::GitHubPullRequest { delivery_id, .. } => Some(&delivery_id),
            QueueItem::GracefulShutdown => None,
        }
    }
}
