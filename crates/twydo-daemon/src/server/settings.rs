//! `settings.get`: what a phone's Settings screen shows about its daemon.

use std::sync::Arc;

use serde_json::Value;

use super::Shared;
use super::views::status_summaries;
use crate::PROTOCOL;
use crate::protocol::rpc::{ErrorKind, RpcError};
use crate::protocol::views::SettingsGetResult;

pub fn get(shared: &Arc<Shared>) -> Result<Value, RpcError> {
    let (_, published) = shared.publisher.current();
    let snap = &published.snap;
    let now = shared.clock.now().with_timezone(&snap.tz()).fixed_offset();
    let (sync, reminders) = status_summaries(shared, snap, now);
    let result = SettingsGetResult {
        version: shared.version.clone(),
        protocol: PROTOCOL,
        server: shared.cfg.sync.as_ref().and_then(|s| server_host(&s.url)),
        sync,
        reminders,
    };
    serde_json::to_value(result).map_err(|e| RpcError::refused(ErrorKind::Internal, e.to_string()))
}

/// The host of an http(s) URL, with its port unless it is the scheme's default. Never the
/// userinfo, path, query or fragment: what a phone may show of the CalDAV server.
pub fn server_host(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let (rest, default) = if lower.starts_with("https://") {
        (&url[8..], "443")
    } else if lower.starts_with("http://") {
        (&url[7..], "80")
    } else {
        return None;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    // A port follows the last `:` outside an IPv6 literal's brackets.
    let (host, port) = match hostport.rfind(':') {
        Some(i) if !hostport[i..].contains(']') => (&hostport[..i], Some(&hostport[i + 1..])),
        _ => (hostport, None),
    };
    // Fail closed: anything but a plain name, IPv4 or bracketed IPv6 address is not shown.
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    let ok_host = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(v6) => {
            !v6.is_empty()
                && v6
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.'))
        }
        None => !host.is_empty() && host.chars().all(plain),
    };
    if !ok_host || !port.is_none_or(|p| p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    Some(match port {
        None | Some("") => host.to_string(),
        Some(p) if p == default => host.to_string(),
        Some(p) => format!("{host}:{p}"),
    })
}

#[cfg(test)]
mod tests {
    use super::server_host;

    #[test]
    fn host_only() {
        assert_eq!(
            server_host("https://dav.example.net/dav/"),
            Some("dav.example.net".into())
        );
        assert_eq!(
            server_host("https://user:pw@dav.example.net/x"),
            Some("dav.example.net".into())
        );
        assert_eq!(
            server_host("HTTPS://dav.example.net?q=1"),
            Some("dav.example.net".into())
        );
    }

    #[test]
    fn default_ports_dropped() {
        assert_eq!(
            server_host("https://dav.example.net:443/"),
            Some("dav.example.net".into())
        );
        assert_eq!(
            server_host("http://dav.example.net:80"),
            Some("dav.example.net".into())
        );
    }

    #[test]
    fn other_ports_kept() {
        assert_eq!(
            server_host("https://dav.example.net:8443/"),
            Some("dav.example.net:8443".into())
        );
        assert_eq!(
            server_host("http://dav.example.net:443/"),
            Some("dav.example.net:443".into())
        );
        assert_eq!(server_host("http://[::1]:5232/"), Some("[::1]:5232".into()));
    }

    #[test]
    fn not_a_url() {
        assert_eq!(server_host(""), None);
        assert_eq!(server_host("not a url"), None);
        assert_eq!(server_host("https:///dav"), None);
        assert_eq!(server_host("ftp://dav.example.net"), None);
        assert_eq!(server_host("https://dav.example.net%2Fx/"), None);
        assert_eq!(server_host("https://dav.example.net:8o/"), None);
    }
}
