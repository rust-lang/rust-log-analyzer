use super::QueueItem;

use crate::rla;
use clap;
use futures::{future, Future, Stream};
use hyper::server::{Request, Response, Service};
use hyper::{self, StatusCode};
use serde_json;
use std::env;
use std::sync;

header! { (XGitHubEvent, "X-GitHub-Event") => [String] }
header! { (XHubSignature, "X-Hub-Signature") => [String] }

#[derive(Clone)]
pub struct RlaService {
    github_webhook_secret: Option<Vec<u8>>,
    reject_unverified_webhooks: bool,
    queue: sync::mpsc::Sender<QueueItem>,
}

impl RlaService {
    pub fn new(
        args: &clap::ArgMatches,
        queue: sync::mpsc::Sender<QueueItem>,
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

        let reject_unverified_webhooks = args.is_present("webhook-verify");

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

    fn handle_webhook(&self, event: &str, headers: &hyper::Headers, body: &[u8]) -> StatusCode {
        if let Some(ref secret) = self.github_webhook_secret {
            let sig = headers
                .get::<XHubSignature>()
                .map(|&XHubSignature(ref sig)| sig.as_ref());

            if let Err(e) = rla::github::verify_webhook_signature(secret, sig, body) {
                if self.reject_unverified_webhooks {
                    error!("Rejecting web hook with invalid signature: {}", e);
                    return StatusCode::Forbidden;
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
                        return StatusCode::BadRequest;
                    }
                };

                match self.queue.send(QueueItem::GitHubStatus(payload)) {
                    Ok(()) => StatusCode::Ok,
                    Err(e) => {
                        error!("Failed to queue payload: {}", e);
                        StatusCode::InternalServerError
                    }
                }
            }
            "check_run" => {
                let payload = match serde_json::from_slice(body) {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to decode 'check_run' web hook payload: {}", e);
                        return StatusCode::BadRequest;
                    }
                };

                match self.queue.send(QueueItem::GitHubCheckRun(payload)) {
                    Ok(()) => StatusCode::Ok,
                    Err(e) => {
                        error!("Failed to queue payload: {}", e);
                        StatusCode::InternalServerError
                    }
                }
            }
            "issue_comment" => {
                debug!("Ignoring 'issue_comment' event.");
                StatusCode::Ok
            }
            _ => {
                warn!("Unexpected '{}' event.", event);
                StatusCode::BadRequest
            }
        }
    }
}

impl Service for RlaService {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;

    type Future = Box<Future<Item = Self::Response, Error = hyper::Error>>;

    fn call(&self, req: Request) -> Self::Future {
        let (method, uri, _version, headers, body) = req.deconstruct();

        let handler: Box<Future<Item = StatusCode, Error = hyper::Error> + 'static> =
            if let Some(XGitHubEvent(ev)) = headers.get().cloned() {
                if method != hyper::Method::Post {
                    warn!("Unexpected web hook method '{}'.", method);
                    Box::new(future::ok(StatusCode::BadRequest))
                } else if uri.path() != "/" {
                    warn!("Unexpected web hook path '{}'.", uri.path());
                    Box::new(future::ok(StatusCode::BadRequest))
                } else {
                    let slf = self.clone();
                    Box::new(body.concat2().and_then(move |body: hyper::Chunk| {
                        future::ok(slf.handle_webhook(&ev, &headers, &body))
                    }))
                }
            } else {
                trace!("Ignoring unrecognized request.");
                Box::new(future::ok(StatusCode::BadRequest))
            };

        Box::new(handler.and_then(|code| {
            let mut res = Response::new();
            res.set_status(code);
            Ok(res)
        }))
    }
}
