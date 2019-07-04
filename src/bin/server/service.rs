use super::QueueItem;

use crate::rla;
use futures::{future, Future, Stream};
use hyper::{self, Body, StatusCode};
use hyper::{Request, Response};
use serde_json;
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

    fn handle_webhook(&self, event: &str, headers: &hyper::HeaderMap, body: &[u8]) -> StatusCode {
        if let Some(ref secret) = self.github_webhook_secret {
            let sig = headers.get("X-Hub-Signature");

            let sig = sig.and_then(|s| s.to_str().ok());
            if let Err(e) = rla::github::verify_webhook_signature(secret, sig, body) {
                if self.reject_unverified_webhooks {
                    error!("Rejecting web hook with invalid signature: {}", e);
                    return StatusCode::FORBIDDEN;
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
                        return StatusCode::BAD_REQUEST;
                    }
                };

                match self.queue.send(QueueItem::GitHubStatus(payload)) {
                    Ok(()) => StatusCode::OK,
                    Err(e) => {
                        error!("Failed to queue payload: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                }
            }
            "check_run" => {
                let payload = match serde_json::from_slice(body) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to decode 'check_run' web hook payload: {}", e);
                        return StatusCode::BAD_REQUEST;
                    }
                };

                match self.queue.send(QueueItem::GitHubCheckRun(payload)) {
                    Ok(()) => StatusCode::OK,
                    Err(e) => {
                        error!("Failed to queue payload: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                }
            }
            "issue_comment" => {
                debug!("Ignoring 'issue_comment' event.");
                StatusCode::OK
            }
            _ => {
                warn!("Unexpected '{}' event.", event);
                StatusCode::BAD_REQUEST
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
        let handler: Box<dyn Future<Item = StatusCode, Error = hyper::Error> + Send + 'static> =
            if let Some(ev) = req.headers.get("X-GitHub-Event").cloned() {
                if req.method != hyper::Method::POST {
                    warn!("Unexpected web hook method '{}'.", req.method);
                    Box::new(future::ok(StatusCode::BAD_REQUEST))
                } else if req.uri.path() != "/" {
                    warn!("Unexpected web hook path '{}'.", req.uri.path());
                    Box::new(future::ok(StatusCode::BAD_REQUEST))
                } else {
                    let slf = self.clone();
                    Box::new(body.concat2().and_then(move |body: hyper::Chunk| {
                        future::ok(slf.handle_webhook(ev.to_str().unwrap(), &req.headers, &body))
                    }))
                }
            } else {
                trace!("Ignoring unrecognized request.");
                Box::new(future::ok(StatusCode::BAD_REQUEST))
            };

        Box::new(handler.and_then(|code| {
            let mut res = Response::new(Body::empty());
            *res.status_mut() = code;
            Ok(res)
        }))
    }
}
