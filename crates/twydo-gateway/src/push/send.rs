//! Sending one reminder to every stored subscription (spec section 7.3).
//!
//! Logs and errors never carry the title, the body or the endpoint path: only the device
//! name, the endpoint's host and a status.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chrono::{DateTime, Utc};
use ureq::Agent;
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig, parse_pem};

use super::crypto::{Payload, encrypt, padded, vapid_header};
use crate::state::Store;

/// What one push service said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Accepted,
    /// The subscription is gone for good (the status).
    Dead(u16),
    /// Worth retrying; the reason holds no endpoint.
    Transient(String),
}

pub fn classify(r: Result<u16, String>) -> Answer {
    match r {
        Ok(200..=299) => Answer::Accepted,
        Ok(s @ (400 | 401 | 403 | 404 | 410)) => Answer::Dead(s),
        Ok(s) => Answer::Transient(format!("status {s}")),
        Err(e) => Answer::Transient(e),
    }
}

/// `Ok` when nothing needs a retry and at least one phone took the message.
pub fn verdict(answers: &[(String, Answer)]) -> Result<(), String> {
    let transient: Vec<_> = answers
        .iter()
        .filter_map(|(d, a)| match a {
            Answer::Transient(r) => Some((d, r)),
            _ => None,
        })
        .collect();
    if let Some((device, reason)) = transient.first() {
        return Err(format!(
            "push failed for {} of {} phones: {device}: {reason}",
            transient.len(),
            answers.len()
        ));
    }
    if answers.iter().any(|(_, a)| *a == Answer::Accepted) {
        Ok(())
    } else {
        Err("no phone has reminders turned on".into())
    }
}

pub struct PushOptions {
    /// Extra trust anchors (PEM) instead of the platform's.
    pub ca_file: Option<PathBuf>,
    /// The whole push, all phones together.
    pub deadline: Duration,
}

impl Default for PushOptions {
    fn default() -> Self {
        PushOptions {
            ca_file: None,
            deadline: Duration::from_secs(8),
        }
    }
}

fn agent(ca_file: Option<&PathBuf>) -> Result<Agent, String> {
    let roots = match ca_file {
        Some(path) => {
            let pem = std::fs::read(path).map_err(|_| "cannot read the CA file".to_string())?;
            let certs: Vec<Certificate<'static>> = parse_pem(&pem)
                .filter_map(|i| match i {
                    Ok(PemItem::Certificate(c)) => Some(Ok(c)),
                    Ok(_) => None,
                    Err(_) => Some(Err(())),
                })
                .collect::<Result<_, _>>()
                .map_err(|_| "invalid CA file".to_string())?;
            if certs.is_empty() {
                return Err("invalid CA file".into());
            }
            RootCerts::Specific(certs.into())
        }
        None => RootCerts::PlatformVerifier,
    };
    Ok(Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .user_agent(format!("tdo/{}", env!("CARGO_PKG_VERSION")))
        .tls_config(TlsConfig::builder().root_certs(roots).build())
        .build()
        .into())
}

fn host(endpoint: &str) -> &str {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    authority.rsplit('@').next().unwrap_or(authority)
}

fn decode(s: &str) -> Result<Vec<u8>, String> {
    B64.decode(s.trim_end_matches('='))
        .map_err(|_| INVALID.to_string())
}

struct Job<'a> {
    agent: &'a Agent,
    endpoint: &'a str,
    p256dh: &'a str,
    auth: &'a str,
    plain: &'a [u8],
    authorization: Result<String, String>,
    start: Instant,
    deadline: Duration,
}

const INVALID: &str = "invalid subscription";

/// Fails with `INVALID` when the stored subscription can never be sent to.
fn post(job: &Job) -> Result<u16, String> {
    let authorization = job.authorization.clone().map_err(|_| INVALID.to_string())?;
    let (p256dh, auth) = (decode(job.p256dh)?, decode(job.auth)?);
    if auth.len() != 16 {
        return Err(INVALID.into());
    }
    let body = encrypt(job.plain, &p256dh, &auth).map_err(|e| {
        if e == "invalid subscription key" {
            INVALID.to_string()
        } else {
            e
        }
    })?;
    let remaining = job.deadline.saturating_sub(job.start.elapsed());
    if remaining.is_zero() {
        return Err("timed out".into());
    }
    let response = job
        .agent
        .post(job.endpoint)
        .header("TTL", "86400")
        .header("Urgency", "high")
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Authorization", authorization)
        .config()
        .timeout_global(Some(remaining))
        .build()
        .send(&body[..])
        .map_err(|e| match e {
            ureq::Error::Timeout(_) => "timed out".to_string(),
            _ => "request failed".to_string(),
        })?;
    Ok(response.status().as_u16())
}

/// Sends `payload` to every subscription, drops the dead ones and returns the verdict.
pub fn push(
    store: &Store,
    payload: &Payload,
    opts: &PushOptions,
    now: DateTime<Utc>,
    log: &mut dyn Write,
) -> Result<(), String> {
    let vapid = store.vapid().map_err(|e| e.to_string())?.ok_or_else(|| {
        "the gateway has no push key yet; start tdo gateway serve with --origin".to_string()
    })?;
    let subscriptions = store.subscriptions().map_err(|e| e.to_string())?;
    if subscriptions.is_empty() {
        return verdict(&[]);
    }
    let agent = agent(opts.ca_file.as_ref())?;
    let plain = padded(payload);
    let start = Instant::now();
    let answers: Vec<(String, Answer)> = std::thread::scope(|scope| {
        let handles: Vec<_> = subscriptions
            .iter()
            .map(|(_, sub)| {
                let job = Job {
                    agent: &agent,
                    endpoint: &sub.endpoint,
                    p256dh: &sub.p256dh,
                    auth: &sub.auth,
                    plain: &plain,
                    authorization: vapid_header(
                        &vapid.key,
                        &sub.endpoint,
                        &vapid.subject,
                        now.timestamp(),
                    ),
                    start,
                    deadline: opts.deadline,
                };
                scope.spawn(move || post(&job))
            })
            .collect();
        subscriptions
            .iter()
            .zip(handles)
            .map(|((device, sub), handle)| {
                let result = handle.join().unwrap_or_else(|_| Err("push crashed".into()));
                let _ = writeln!(
                    log,
                    "push {} {} {}",
                    device.name,
                    host(&sub.endpoint),
                    match &result {
                        Ok(status) => status.to_string(),
                        Err(reason) => reason.clone(),
                    }
                );
                let answer = match &result {
                    Err(reason) if reason == INVALID => Answer::Dead(0),
                    _ => classify(result),
                };
                if matches!(answer, Answer::Dead(_))
                    && store.remove_dead(&device.id, &sub.endpoint).is_err()
                {
                    let _ = writeln!(
                        log,
                        "push {} could not remove a dead subscription",
                        device.name
                    );
                }
                (device.name.clone(), answer)
            })
            .collect()
    });
    verdict(&answers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Answer {
        Answer::Transient(s.into())
    }

    #[test]
    fn classify_table() {
        for s in [201, 200, 204] {
            assert_eq!(classify(Ok(s)), Answer::Accepted);
        }
        for s in [400, 401, 403, 404, 410] {
            assert_eq!(classify(Ok(s)), Answer::Dead(s));
        }
        for s in [413, 429, 500, 503] {
            assert_eq!(classify(Ok(s)), t(&format!("status {s}")));
        }
        assert_eq!(classify(Err("timed out".into())), t("timed out"));
    }

    fn named(a: Answer) -> (String, Answer) {
        ("Pixel".into(), a)
    }

    #[test]
    fn verdict_cases() {
        let none = "no phone has reminders turned on";
        assert_eq!(verdict(&[]), Err(none.into()));
        assert_eq!(
            verdict(&[named(Answer::Dead(410)), named(Answer::Dead(404))]),
            Err(none.into())
        );
        assert_eq!(verdict(&[named(Answer::Accepted)]), Ok(()));
        assert_eq!(
            verdict(&[named(Answer::Accepted), named(Answer::Dead(410))]),
            Ok(())
        );
        assert_eq!(
            verdict(&[named(Answer::Accepted), ("Old".into(), t("status 500"))]),
            Err("push failed for 1 of 2 phones: Old: status 500".into())
        );
    }

    #[test]
    fn host_has_no_path() {
        assert_eq!(host("https://push.example:8443/a/b?c"), "push.example:8443");
        assert_eq!(host("https://user:pw@push.example/a"), "push.example");
    }
}
