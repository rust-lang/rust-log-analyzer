use super::QueueItem;

use crate::rla;
use futures::{future, Future, Stream};
use hyper::{self, Body, Method, StatusCode};
use hyper::{Request, Response};
use serde_json;
use std::env;

type ResponseFuture = Box<dyn Future<Item = Response<Body>, Error = hyper::Error> + Send + 'static>;

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

    fn handle_webhook(
        &self,
        event: &str,
        headers: &hyper::HeaderMap,
        body: &[u8],
    ) -> ResponseFuture {
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

        match event {
            "status" => {
                let payload = match serde_json::from_slice(body) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to decode 'status' web hook payload: {}", e);
                        return reply(StatusCode::BAD_REQUEST, "Failed to decode payload.\n");
                    }
                };

                match self.queue.send(QueueItem::GitHubStatus(payload)) {
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
            "check_run" => {
                let payload = match serde_json::from_slice(body) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to decode 'check_run' web hook payload: {}", e);
                        return reply(StatusCode::BAD_REQUEST, "Failed to decode payload.\n");
                    }
                };

                match self.queue.send(QueueItem::GitHubCheckRun(payload)) {
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
            "issue_comment" => {
                debug!("Ignoring 'issue_comment' event.");
                reply(StatusCode::OK, "Event ignored.\n")
            }
            _ => {
                warn!("Unexpected '{}' event.", event);
                reply(StatusCode::BAD_REQUEST, "Unexpected event.\n")
            }
        }
    }
}

impl RlaService {
    pub fn call(
        &self,
        req: Request<Body>,
    ) -> Box<dyn Future<Item = Response<Body>, Error = hyper::Error> + Send> {
        let (req, body) = req.into_parts();
        info!("request: {} {}", req.method, req.uri.path());
        match (req.method.clone(), req.uri.path()) {
            (Method::GET, "/") => reply(StatusCode::OK, "Rust Log Analyzer is running.\n"),
            (Method::POST, "/") => {
                if let Some(ev) = req.headers.get("X-GitHub-Event").cloned() {
                    let slf = self.clone();
                    Box::new(body.concat2().and_then(move |body: hyper::Chunk| {
                        slf.handle_webhook(ev.to_str().unwrap(), &req.headers, &body)
                    }))
                } else {
                    reply(StatusCode::BAD_REQUEST, "Missing X-GitHub-Event header.\n")
                }
            }
            (_, "/") => reply(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed.\n"),
            _ => reply(StatusCode::NOT_FOUND, "Not found.\n"),
        }
    }
}

fn reply(status: StatusCode, body: &'static str) -> ResponseFuture {
    info!("response: {} {:?}", status.as_u16(), body.trim());
    let mut resp = Response::new(Body::from(body));
    *resp.status_mut() = status;
    Box::new(future::ok(resp))
}
