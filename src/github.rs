use super::Result;
use hyper::header;
use reqwest;
use std::env;
use std::str;
use std::time::Duration;

const TIMEOUT_SECS: u64 = 15;
static ACCEPT_VERSION: &str = "application/vnd.github.v3+json";
static API_BASE: &str = "https://api.github.com";

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
}

#[derive(Deserialize)]
pub struct Commit {
    pub message: String,
}

pub struct Client {
    auth: (String, String),
    internal: reqwest::Client,
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
    pub external_id: String,
    pub details_url: String,
    pub app: App,
}

#[derive(Deserialize)]
pub struct App {
    pub id: u64,
}

#[derive(Deserialize)]
pub struct Repository {
    pub full_name: String,
}

impl Client {
    pub fn new() -> Result<Client> {
        let user = env::var("GITHUB_USER")
            .map_err(|e| format_err!("Could not read GITHUB_USER: {}", e))?;
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

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .referer(false)
            .timeout(Some(Duration::from_secs(TIMEOUT_SECS)))
            .build()?;

        Ok(Client {
            internal: client,
            auth: (user, token),
        })
    }

    pub fn query_pr(&self, repo: &str, pr_id: u32) -> Result<Pr> {
        let mut resp = self
            .internal
            .get(format!("{}/repos/{}/pulls/{}", API_BASE, repo, pr_id).as_str())
            .basic_auth(&self.auth.0, Some(&self.auth.1))
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
            .basic_auth(&self.auth.0, Some(&self.auth.1))
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
            .basic_auth(&self.auth.0, Some(&self.auth.1))
            .json(&Comment { body: comment })
            .send()?;
        if !resp.status().is_success() {
            bail!("Posting comment failed: {:?}", resp);
        }

        Ok(())
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
