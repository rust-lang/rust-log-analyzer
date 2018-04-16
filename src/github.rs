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
}

#[derive(Deserialize)]
pub struct Pr {
    pub head: PrCommitRef,
}

#[derive(Deserialize)]
pub struct PrCommitRef {
    pub sha: String,
}

pub struct Client {
    internal: reqwest::Client,
}

#[derive(Serialize)]
struct Comment<'a> {
    body: &'a str,
}

impl Client {
    pub fn new() -> Result<Client> {
        let user = env::var("GITHUB_USER")
            .map_err(|e| format_err!("Could not read GITHUB_USER: {}", e))?;
        let token = env::var("GITHUB_TOKEN")
            .map_err(|e| format_err!("Could not read GITHUB_TOKEN: {}", e))?;


        let mut headers = header::Headers::new();
        headers.set(header::Authorization(header::Basic {
            username: user,
            password: Some(token),
        }));
        headers.set(header::Accept(vec![ACCEPT_VERSION.parse()?]));
        headers.set(header::UserAgent::new(super::USER_AGENT));

        let client = reqwest::Client::builder().default_headers(headers)
            .referer(false)
            .timeout(Some(Duration::from_secs(TIMEOUT_SECS)))
            .build()?;

        Ok(Client { internal: client })
    }

    pub fn query_pr(&self, repo: &str, pr_id: u32) -> Result<Pr> {
        let mut resp = self.internal.get(format!("{}/repos/{}/pulls/{}", API_BASE, repo, pr_id).as_str()).send()?;

        if !resp.status().is_success() {
            bail!("Querying PR failed: {:?}", resp);
        }

        Ok(resp.json()?)
    }

    pub fn post_comment(&self, repo: &str, issue_id: u32, comment: &str) -> Result<()> {
        let resp = self.internal.post(format!("{}/repos/{}/issues/{}/comments", API_BASE, repo, issue_id).as_str())
            .json(&Comment { body: comment })
            .send()?;
        if !resp.status().is_success() {
            bail!("Posting comment failed: {:?}", resp);
        }

        Ok(())
    }
}

pub fn verify_webhook_signature(secret: &[u8], signature: Option<&str>, body: &[u8]) -> Result<()> {
    use hex;
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
