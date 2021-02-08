use super::Result;
use crate::ci::Outcome;
use hyper::header;
use reqwest;
use serde::{de::DeserializeOwned, Serialize};
use std::env;
use std::str;
use std::time::Duration;

const TIMEOUT_SECS: u64 = 15;
static ACCEPT_VERSION: &str = "application/vnd.github.v3+json";
static API_BASE: &str = "https://api.github.com";

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    Queued,
    InProgress,
    Completed,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildConclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    TimedOut,
    ActionRequired,
    Skipped,
}

#[derive(Deserialize, Debug)]
pub struct BuildOutcome {
    status: BuildStatus,
    conclusion: Option<BuildConclusion>,
}

impl Outcome for BuildOutcome {
    fn is_finished(&self) -> bool {
        self.status == BuildStatus::Completed
    }

    fn is_passed(&self) -> bool {
        self.is_finished() && self.conclusion == Some(BuildConclusion::Success)
    }

    fn is_failed(&self) -> bool {
        self.is_finished() && self.conclusion == Some(BuildConclusion::Failure)
    }
}

#[derive(Deserialize)]
pub struct CommitStatusEvent {
    pub target_url: String,
    pub context: String,
    pub repository: Repository,
}

#[derive(Deserialize)]
pub struct Pr {
    pub head: PrCommitRef,
}

#[derive(Deserialize)]
pub struct PrCommitRef {
    pub sha: String,
}

#[derive(Deserialize)]
pub struct CommitMeta {
    pub commit: Commit,
    pub parents: Vec<CommitParent>,
}

#[derive(Deserialize)]
pub struct Commit {
    pub message: String,
}

#[derive(Deserialize)]
pub struct CommitParent {
    pub sha: String,
}

#[derive(Serialize)]
struct Comment<'a> {
    body: &'a str,
}

#[derive(Deserialize)]
pub struct CheckRunEvent {
    pub check_run: CheckRun,
    pub repository: Repository,
}

#[derive(Deserialize)]
pub struct CheckRun {
    pub url: String,
    pub external_id: String,
    pub details_url: String,
    pub app: App,
    pub check_suite: CheckSuite,
    #[serde(flatten)]
    pub outcome: BuildOutcome,
}

#[derive(Deserialize)]
pub struct CheckSuite {
    pub id: u64,
    pub url: String,
}

#[derive(Deserialize)]
pub struct App {
    pub id: u64,
}

#[derive(Deserialize)]
pub struct Repository {
    pub full_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestAction {
    Opened,
    Edited,
    Closed,
    Assigned,
    Unassigned,
    ReviewRequested,
    ReviewRequestRemoved,
    ReadyForReview,
    Labeled,
    Unlabeled,
    Synchronize,
    Locked,
    Unlocked,
    Reopened,
}

#[derive(Deserialize)]
pub struct PullRequestEvent {
    pub action: PullRequestAction,
    pub number: u32,
    pub repository: Repository,
}

#[derive(Deserialize)]
struct GraphResponse<T> {
    data: T,
    #[serde(default)]
    errors: Vec<GraphError>,
}

#[derive(Debug, Deserialize)]
struct GraphError {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphPageInfo {
    end_cursor: Option<String>,
}

pub struct Client {
    internal: reqwest::Client,
}

impl Client {
    pub fn new() -> Result<Client> {
        let token = env::var("GITHUB_TOKEN")
            .map_err(|e| format_err!("Could not read GITHUB_TOKEN: {}", e))?;

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static(ACCEPT_VERSION),
        );
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static(super::USER_AGENT),
        );
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("token {}", token))?,
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .referer(false)
            .timeout(Some(Duration::from_secs(TIMEOUT_SECS)))
            .build()?;

        Ok(Client { internal: client })
    }

    pub fn query_pr(&self, repo: &str, pr_id: u32) -> Result<Pr> {
        let mut resp = self
            .internal
            .get(format!("{}/repos/{}/pulls/{}", API_BASE, repo, pr_id).as_str())
            .send()?;

        if !resp.status().is_success() {
            bail!("Querying PR failed: {:?}", resp);
        }

        Ok(resp.json()?)
    }

    pub fn query_commit(&self, repo: &str, sha: &str) -> Result<CommitMeta> {
        let mut resp = self
            .internal
            .get(format!("{}/repos/{}/commits/{}", API_BASE, repo, sha).as_str())
            .send()?;

        if !resp.status().is_success() {
            bail!("Querying commit failed: {:?}", resp);
        }

        Ok(resp.json()?)
    }

    pub fn post_comment(&self, repo: &str, issue_id: u32, comment: &str) -> Result<()> {
        let resp = self
            .internal
            .post(format!("{}/repos/{}/issues/{}/comments", API_BASE, repo, issue_id).as_str())
            .json(&Comment { body: comment })
            .send()?;
        if !resp.status().is_success() {
            bail!("Posting comment failed: {:?}", resp);
        }

        Ok(())
    }

    pub fn hide_own_comments(&self, repo: &str, pull_request_id: u32) -> Result<()> {
        const QUERY: &str = "query($owner: String!, $repo: String!, $pr: Int!, $cursor: String) {
                repository(owner: $owner, name: $repo) {
                    pullRequest(number: $pr) {
                        comments(first: 100, after: $cursor) {
                            nodes {
                                id
                                isMinimized
                                viewerDidAuthor
                            }
                            pageInfo {
                                endCursor
                            }
                        }
                    }
                }
            }";

        #[derive(Debug, Deserialize)]
        struct Response {
            repository: ResponseRepo,
        }
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ResponseRepo {
            pull_request: ResponsePR,
        }
        #[derive(Debug, Deserialize)]
        struct ResponsePR {
            comments: ResponseComments,
        }
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ResponseComments {
            nodes: Vec<ResponseComment>,
            page_info: GraphPageInfo,
        }
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ResponseComment {
            id: String,
            is_minimized: bool,
            viewer_did_author: bool,
        }

        debug!("started hiding comments in {}#{}", repo, pull_request_id);

        let (owner, repo) = if let Some(mid) = repo.find('/') {
            let split = repo.split_at(mid);
            (split.0, split.1.trim_start_matches('/'))
        } else {
            bail!("invalid repository name: {}", repo);
        };

        let mut comments = Vec::new();
        let mut cursor = None;
        loop {
            let mut resp: Response = self.graphql(
                QUERY,
                serde_json::json!({
                    "owner": owner,
                    "repo": repo,
                    "pr": pull_request_id,
                    "cursor": cursor,
                }),
            )?;
            cursor = resp.repository.pull_request.comments.page_info.end_cursor;
            comments.append(&mut resp.repository.pull_request.comments.nodes);

            if cursor.is_none() {
                break;
            }
        }

        for comment in &comments {
            if comment.viewer_did_author && !comment.is_minimized {
                self.hide_comment(&comment.id, "OUTDATED")?;
            }
        }
        Ok(())
    }

    fn hide_comment(&self, node_id: &str, reason: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct MinimizeData {}

        const MINIMIZE: &str = "mutation($node_id: ID!, $reason: ReportedContentClassifiers!) {
            minimizeComment(input: {subjectId: $node_id, classifier: $reason}) {
                __typename
            }
        }";

        trace!("hiding comment {}", node_id);

        self.graphql::<Option<MinimizeData>, _>(
            MINIMIZE,
            serde_json::json!({
                "node_id": node_id,
                "reason": reason,
            }),
        )?;
        Ok(())
    }

    pub fn internal(&self) -> &reqwest::Client {
        &self.internal
    }

    fn graphql<T: DeserializeOwned, V: Serialize>(&self, query: &str, variables: V) -> Result<T> {
        #[derive(Serialize)]
        struct GraphPayload<'a, V> {
            query: &'a str,
            variables: V,
        }

        let response: GraphResponse<T> = self
            .internal
            .post(&format!("{}/graphql", API_BASE))
            .json(&GraphPayload { query, variables })
            .send()?
            .error_for_status()?
            .json()?;

        if response.errors.is_empty() {
            Ok(response.data)
        } else {
            dbg!(&response.errors);
            bail!("GraphQL query failed: {}", response.errors[0].message);
        }
    }
}

pub fn verify_webhook_signature(secret: &[u8], signature: Option<&str>, body: &[u8]) -> Result<()> {
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    let signature = match signature {
        Some(sig) => sig,
        None => bail!("Missing signature."),
    };

    if !signature.starts_with("sha1=") {
        bail!("Invalid signature format.");
    }

    let signature = &signature[5..];

    let decoded_signature = hex::decode(signature)?;

    let mut mac = Hmac::<Sha1>::new_varkey(secret).unwrap();
    mac.input(body);

    if mac.result().is_equal(&decoded_signature) {
        Ok(())
    } else {
        bail!("Signature mismatch.");
    }
}
