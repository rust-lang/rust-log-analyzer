use super::QueueItem;

use anyhow::bail;
use hyper::{Body, Method, StatusCode};
use hyper::{Request, Response};
use std::env;

#[derive(Clone)]
pub struct RlaService {
    github_webhook_secret: Option<Vec<u8>>,
    reject_unverified_webhooks: bool,
    queue: crossbeam::channel::Sender<QueueItem>,
}

impl RlaService {
    pub fn new(
        reject_unverified_webhooks: bool,
        queue: crossbeam::channel::Sender<QueueItem>,
    ) -> rla::Result<RlaService> {
        let github_webhook_secret = match env::var("GITHUB_WEBHOOK_SECRET") {
            Err(env::VarError::NotPresent) => None,
            Err(env::VarError::NotUnicode(_)) => {
                bail!("GITHUB_WEBHOOK_SECRET contained non-UTF-8 data.")
            }
            Ok(s) => {
                if !s.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    bail!("Only alphanumeric ASCII characters are allowed in GITHUB_WEBHOOK_SECRET at this time.");
                }

                Some(s.into_bytes())
            }
        };

        if reject_unverified_webhooks {
            if github_webhook_secret.is_none() {
                bail!("Web hook verification was requested but no valid GITHUB_WEBHOOK_SECRET was specified.");
            }
        }

        Ok(RlaService {
            github_webhook_secret,
            reject_unverified_webhooks,
            queue,
        })
    }

    async fn handle_webhook(
        &self,
        event: &str,
        headers: &hyper::HeaderMap,
        body: &[u8],
    ) -> Result<Response<Body>, hyper::Error> {
        if let Some(ref secret) = self.github_webhook_secret {
            let sig = headers.get("X-Hub-Signature");

            let sig = sig.and_then(|s| s.to_str().ok());
            if let Err(e) = rla::github::verify_webhook_signature(secret, sig, body) {
                if self.reject_unverified_webhooks {
                    error!("Rejecting web hook with invalid signature: {}", e);
                    return reply(StatusCode::FORBIDDEN, "Invalid signature.\n");
                }

                warn!("Processing web hook with invalid signature: {}", e);
            }
        };

        let delivery_header = headers
            .get("X-GitHub-Delivery")
            .and_then(|s| s.to_str().ok())
            .map(|s| s.to_string());
        let delivery_id = if let Some(id) = delivery_header {
            id
        } else {
            return reply(StatusCode::BAD_REQUEST, "Missing delivery ID.\n");
        };

        let item = match event {
            "status" => {
                let payload = match serde_json::from_slice(body) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to decode 'status' web hook payload: {}", e);
                        return reply(StatusCode::BAD_REQUEST, "Failed to decode payload.\n");
                    }
                };
                QueueItem::GitHubStatus {
                    payload,
                    delivery_id,
                }
            }
            "check_run" => {
                let payload = match serde_json::from_slice(body) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to decode 'check_run' web hook payload: {}", e);
                        return reply(StatusCode::BAD_REQUEST, "Failed to decode payload.\n");
                    }
                };

                QueueItem::GitHubCheckRun {
                    payload,
                    delivery_id,
                }
            }
            "pull_request" => match serde_json::from_slice(body) {
                Ok(payload) => QueueItem::GitHubPullRequest {
                    payload,
                    delivery_id,
                },
                Err(err) => {
                    error!("Failed to decode 'pull_request' webhook payload: {}", err);
                    return reply(StatusCode::BAD_REQUEST, "Failed to decode payload\n");
                }
            },
            "issue_comment" => {
                debug!("Ignoring 'issue_comment' event.");
                return reply(StatusCode::OK, "Event ignored.\n");
            }
            _ => {
                warn!("Unexpected '{}' event.", event);
                return reply(StatusCode::BAD_REQUEST, "Unexpected event.\n");
            }
        };

        match self.queue.send(item) {
            Ok(()) => reply(StatusCode::OK, "Event processed.\n"),
            Err(e) => {
                error!("Failed to queue payload: {}", e);
                reply(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to process the event.\n",
                )
            }
        }
    }
}

impl RlaService {
    pub async fn call(&self, req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
        let (req, body) = req.into_parts();
        info!("request: {} {}", req.method, req.uri.path());
        match (req.method.clone(), req.uri.path()) {
            (Method::GET, "/") => reply(StatusCode::OK, "Rust Log Analyzer is running.\n"),
            (Method::POST, "/") => {
                if let Some(ev) = req.headers.get("X-GitHub-Event").cloned() {
                    let slf = self.clone();
                    let body = hyper::body::to_bytes(body).await?;
                    slf.handle_webhook(ev.to_str().unwrap(), &req.headers, &body)
                        .await
                } else {
                    reply(StatusCode::BAD_REQUEST, "Missing X-GitHub-Event header.\n")
                }
            }
            (_, "/") => reply(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed.\n"),
            _ => reply(StatusCode::NOT_FOUND, "Not found.\n"),
        }
    }
}

fn reply(status: StatusCode, body: &'static str) -> Result<Response<Body>, hyper::Error> {
    trace!("response: {} {:?}", status.as_u16(), body.trim());
    let mut resp = Response::new(Body::from(body));
    *resp.status_mut() = status;
    Ok(resp)
}
