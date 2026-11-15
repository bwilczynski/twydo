//! CalDAV calls for list operations: the requests a list operation sends to the server.
//!
//! Every request stays on the configured origin, follows no redirect and treats a 3xx as a
//! failure. Refusal codes carry no server output, addresses or credentials.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ureq::Agent;
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig, parse_pem};

use super::call::Refused;
use crate::config::SyncConfig;

const D: &str = "DAV:";
const C: &str = "urn:ietf:params:xml:ns:caldav";
const A: &str = "http://apple.com/ns/ical/";
/// The most one response body may hold.
const MAX_DAV: usize = 4 * 1024 * 1024;
/// The most a delete backup may hold in total.
const MAX_BACKUP: usize = 32 * 1024 * 1024;
const MAX_REQUEST: Duration = Duration::from_secs(30);
const XML: &str = "application/xml";
const XML_UTF8: &str = "application/xml; charset=utf-8";

fn dav() -> Refused {
    Refused::new("dav")
}

// ---------------------------------------------------------------------------------------
// URLs
// ---------------------------------------------------------------------------------------

struct Split<'a> {
    scheme: String,
    netloc: &'a str,
    path: &'a str,
    query: &'a str,
    fragment: &'a str,
}

fn split(url: &str) -> Split<'_> {
    let (scheme, rest) = match url.find("://") {
        Some(i)
            if !url[..i].is_empty()
                && url[..i]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) =>
        {
            (url[..i].to_ascii_lowercase(), &url[i + 1..])
        }
        _ => (String::new(), url),
    };
    let (netloc, rest) = match rest.strip_prefix("//") {
        Some(r) => {
            let end = r.find(['/', '?', '#']).unwrap_or(r.len());
            (&r[..end], &r[end..])
        }
        None => ("", rest),
    };
    let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    Split {
        scheme,
        netloc,
        path,
        query,
        fragment,
    }
}

/// Removes `.` and `..` segments the way `urllib.parse.urljoin` does.
fn normalise(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let mut out: Vec<&str> = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        match *segment {
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
                if last {
                    out.push("");
                }
            }
            "." => {
                if last {
                    out.push("");
                }
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// `urllib.parse.urljoin` for the http(s) references DAV servers use.
fn join(base: &str, href: &str) -> String {
    let b = split(base);
    let h = split(href);
    let own_netloc = !h.scheme.is_empty() || !h.netloc.is_empty();
    let (scheme, netloc, path) = if own_netloc {
        let scheme = if h.scheme.is_empty() {
            b.scheme.clone()
        } else {
            h.scheme.clone()
        };
        (scheme, h.netloc, h.path.to_string())
    } else if h.path.is_empty() {
        (b.scheme.clone(), b.netloc, b.path.to_string())
    } else if h.path.starts_with('/') {
        (b.scheme.clone(), b.netloc, h.path.to_string())
    } else {
        let dir = b.path.rsplit_once('/').map_or("", |(dir, _)| dir);
        (b.scheme.clone(), b.netloc, format!("{dir}/{}", h.path))
    };
    let query = if h.query.is_empty() {
        String::new()
    } else {
        format!("?{}", h.query)
    };
    let fragment = if h.fragment.is_empty() {
        String::new()
    } else {
        format!("#{}", h.fragment)
    };
    // Python's urljoin returns an absolute reference untouched, dot segments included, so
    // `validate_target` still sees them.
    let path = if own_netloc { path } else { normalise(&path) };
    format!("{scheme}://{netloc}{path}{query}{fragment}")
}

fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `id` is a plain collection identity that is safe in a request path.
fn plain(id: &str) -> bool {
    super::plain_id(id) && id.len() <= 255 && !id.chars().any(|c| "/\\%?#:\0\r\n".contains(c))
}

/// A target must be exactly one collection directly below the configured origin.
pub fn validate_target(base: &str, target: &str, id: &str) -> Result<(), Refused> {
    let mapping = || Refused::new("mapping");
    let (b, t) = (split(base), split(target));
    if !matches!(b.scheme.as_str(), "https" | "http")
        || b.scheme != t.scheme
        || b.netloc != t.netloc
        || !t.query.is_empty()
        || !t.fragment.is_empty()
        || t.netloc.contains('@')
        || !plain(id)
    {
        return Err(mapping());
    }
    let path = unquote(t.path);
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let lower = t.path.to_ascii_lowercase();
    if path.contains('%')
        || path.contains('\\')
        || parts.iter().any(|p| matches!(*p, "." | ".."))
        || parts.len() < 3
        || parts[parts.len() - 1] != id
        || lower.contains("%2f")
        || lower.contains("%5c")
    {
        return Err(mapping());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// XML
// ---------------------------------------------------------------------------------------

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn is(node: &Node, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(ns) && node.tag_name().name() == name
}

fn kids<'a, 'i>(
    node: Node<'a, 'i>,
    ns: &'static str,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(move |c| is(c, ns, name))
}

fn kid<'a, 'i>(node: Node<'a, 'i>, ns: &'static str, name: &'static str) -> Option<Node<'a, 'i>> {
    kids(node, ns, name).next()
}

/// ElementTree's `findtext`: `None` when absent, `""` for an empty element.
fn findtext<'a>(node: Node<'a, '_>, ns: &'static str, name: &'static str) -> Option<&'a str> {
    kid(node, ns, name).map(|k| k.text().unwrap_or(""))
}

/// The status code of a `propstat`: the second word of its status line.
fn status_code<'a>(propstat: Node<'a, '_>) -> Option<&'a str> {
    findtext(propstat, D, "status")
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
}

fn utf8<'a>(raw: &'a [u8], code: &'static str) -> Result<&'a str, Refused> {
    std::str::from_utf8(raw).map_err(|_| Refused::new(code))
}

fn parse<'a>(text: &'a str, code: &'static str) -> Result<Document<'a>, Refused> {
    Document::parse(text).map_err(|_| Refused::new(code))
}

type Tag<'a> = (&'a str, &'a str);
type Props<'a, 'i> = HashMap<Tag<'a>, Node<'a, 'i>>;

fn tag<'a>(node: Node<'a, '_>) -> Tag<'a> {
    (
        node.tag_name().namespace().unwrap_or(""),
        node.tag_name().name(),
    )
}

/// The properties of the one response, from its `200` propstats only.
fn successful_properties<'a, 'i>(
    doc: &'a Document<'i>,
    target: &str,
) -> Result<Props<'a, 'i>, Refused> {
    let root = doc.root_element();
    let rows: Vec<_> = kids(root, D, "response").collect();
    if !is(&root, D, "multistatus") || rows.len() != 1 {
        return Err(dav());
    }
    let href = findtext(rows[0], D, "href").unwrap_or("");
    if href.is_empty() || join(target, href).trim_end_matches('/') != target.trim_end_matches('/') {
        return Err(dav());
    }
    let mut result = HashMap::new();
    for propstat in kids(rows[0], D, "propstat") {
        if status_code(propstat) != Some("200") {
            continue;
        }
        let prop = kid(propstat, D, "prop").ok_or_else(dav)?;
        for value in prop.children().filter(Node::is_element) {
            if result.insert(tag(value), value).is_some() {
                return Err(dav());
            }
        }
    }
    Ok(result)
}

fn multistatus<'a, 'i>(
    doc: &'a Document<'i>,
    require_success: bool,
) -> Result<Vec<Node<'a, 'i>>, Refused> {
    let inventory = || Refused::new("inventory");
    let root = doc.root_element();
    if !is(&root, D, "multistatus") {
        return Err(inventory());
    }
    let rows: Vec<_> = kids(root, D, "response").collect();
    if rows.is_empty()
        || rows
            .iter()
            .any(|row| kid(*row, D, "propstat").is_none() || kid(*row, D, "status").is_some())
        || (require_success
            && rows
                .iter()
                .any(|row| kids(*row, D, "propstat").any(|ps| status_code(ps) != Some("200"))))
    {
        return Err(inventory());
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Properties {
    pub name: Option<String>,
    pub color: Option<String>,
}

/// The fields a save changes; at least one must be set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Changes {
    pub name: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub href: String,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub href: String,
    pub etag: String,
    pub bytes: String,
    pub sha256: String,
}

/// A list's raw properties and every resource, as a backup stores them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Captured {
    pub format: u32,
    pub properties: String,
    pub resources: Vec<Resource>,
}

/// `#RRGGBB` or `#RRGGBBAA`, canonicalised to upper-case `#RRGGBB`.
pub fn canonical_color(value: &str) -> Option<String> {
    let hex = value.strip_prefix('#')?;
    if (hex.len() == 6 || hex.len() == 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(format!("#{}", hex[..6].to_ascii_uppercase()))
    } else {
        None
    }
}

fn same_metadata(color: bool, left: Option<&str>, right: Option<&str>) -> bool {
    if color {
        left.and_then(canonical_color) == right.and_then(canonical_color)
    } else {
        left == right
    }
}

// ---------------------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------------------

struct Reply {
    status: u16,
    etag: Option<String>,
    body: Vec<u8>,
}

pub struct Dav {
    agent: Agent,
    /// The configured URL, as written.
    configured: String,
    /// The configured URL with exactly one trailing slash: where discovery starts.
    root: String,
    auth: String,
    deadline: Instant,
}

/// The most password output accepted; more is refused rather than truncated.
const MAX_PASSWORD: usize = 64 * 1024;

/// Runs `password_command` in its own process group; its stderr is discarded and stdin is
/// null. The group is killed on a timeout and once the command has exited, so a forked
/// process that keeps the pipe open cannot outlive the deadline.
fn password(argv: &[String], deadline: Instant) -> Result<String, Refused> {
    use std::os::unix::process::CommandExt;
    let credentials = || Refused::new("credentials");
    let (program, args) = argv.split_first().ok_or_else(credentials)?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| credentials())?;
    let group = child.id();
    let mut stdout = child.stdout.take().ok_or_else(credentials)?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stdout)
            .take(MAX_PASSWORD as u64 + 1)
            .read_to_end(&mut out);
        let _ = sender.send(out);
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                crate::sync::running::kill_group(group);
                let _ = child.kill();
                let _ = child.wait();
                return Err(Refused::new("timeout"));
            }
        }
    };
    // Whatever the command left behind must not keep the pipe open.
    crate::sync::running::kill_group(group);
    let wait = deadline.saturating_duration_since(Instant::now());
    let out = receiver
        .recv_timeout(wait)
        .map_err(|_| Refused::new("timeout"))?;
    if !status.success() || out.len() > MAX_PASSWORD {
        return Err(credentials());
    }
    // vdirsyncer's command fetch strips newlines from both ends.
    let text = String::from_utf8(out).map_err(|_| credentials())?;
    let text = text.trim_matches('\n');
    if text.is_empty() {
        return Err(credentials());
    }
    Ok(text.to_string())
}

fn agent(sync: &SyncConfig) -> Result<Agent, Refused> {
    let roots = match &sync.ca_file {
        Some(path) => {
            let pem = std::fs::read(path).map_err(|_| dav())?;
            let certificates: Vec<Certificate<'static>> = parse_pem(&pem)
                .filter_map(|item| match item {
                    Ok(PemItem::Certificate(certificate)) => Some(Ok(certificate)),
                    Ok(_) => None,
                    Err(_) => Some(Err(())),
                })
                .collect::<Result<_, _>>()
                .map_err(|_| dav())?;
            if certificates.is_empty() {
                return Err(dav());
            }
            RootCerts::Specific(certificates.into_iter().collect::<Vec<_>>().into())
        }
        None => RootCerts::PlatformVerifier,
    };
    let tls = TlsConfig::builder().root_certs(roots).build();
    Ok(Agent::config_builder()
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .proxy(None)
        .user_agent(format!("tdo/{}", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build()
        .into())
}

/// Runs the password command and prepares a client for `sync`. Nothing is sent yet.
pub fn connect(sync: &SyncConfig, deadline: Instant) -> Result<Dav, Refused> {
    let secret = password(&sync.password_command, deadline)?;
    let auth = format!(
        "Basic {}",
        STANDARD.encode(format!("{}:{secret}", sync.username))
    );
    Ok(Dav {
        agent: agent(sync)?,
        configured: sync.url.clone(),
        root: format!("{}/", sync.url.trim_end_matches('/')),
        auth,
        deadline,
    })
}

impl Dav {
    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<Reply, Refused> {
        let (origin, target) = (split(&self.root), split(url));
        if (&origin.scheme, origin.netloc) != (&target.scheme, target.netloc) {
            return Err(Refused::new("mapping"));
        }
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Refused::new("timeout"));
        }
        let mut builder = ureq::http::Request::builder()
            .method(method)
            .uri(url)
            .header("Authorization", &self.auth);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder
            .body(body.unwrap_or_default().to_vec())
            .map_err(|_| dav())?;
        let request = self
            .agent
            .configure_request(request)
            .timeout_global(Some(left.min(MAX_REQUEST)))
            .build();
        let response = self.agent.run(request).map_err(|error| match error {
            ureq::Error::Timeout(_) => Refused::new("timeout"),
            _ => dav(),
        })?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(dav());
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mut body = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_DAV as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| dav())?;
        if body.len() > MAX_DAV {
            return Err(dav());
        }
        Ok(Reply { status, etag, body })
    }

    fn propfind(&self, url: &str, depth: &str, body: &str) -> Result<Reply, Refused> {
        self.send(
            "PROPFIND",
            url,
            &[("Depth", depth), ("Content-Type", XML)],
            Some(body.as_bytes()),
        )
    }

    /// The principal's URL: `current-user-principal` at the configured URL, else at the
    /// well-known path, else the configured URL itself.
    fn find_principal(&self) -> Result<String, Refused> {
        match self.principal_at(&self.root) {
            Ok(url) => Ok(url),
            Err(Soft::Hard(refused)) => Err(refused),
            Err(Soft::Failed) => {
                let well_known = join(&self.root, "/.well-known/caldav");
                self.principal_at(&well_known).map_err(Soft::refused)
            }
        }
    }

    fn principal_at(&self, url: &str) -> Result<String, Soft> {
        let body = "<propfind xmlns=\"DAV:\"><prop><current-user-principal /></prop></propfind>";
        let reply = self.propfind(url, "0", body).map_err(Soft::Hard)?;
        if reply.status >= 400 {
            return Err(Soft::Failed);
        }
        let text = utf8(&reply.body, "dav").map_err(|_| Soft::Failed)?;
        let doc = parse(text, "dav").map_err(|_| Soft::Failed)?;
        let href = doc
            .root_element()
            .descendants()
            .filter(|n| is(n, D, "current-user-principal"))
            .find_map(|n| kid(n, D, "href"));
        match href {
            None => Ok(url.to_string()),
            Some(href) => {
                let text = href.text().filter(|t| !t.is_empty()).ok_or(Soft::Failed)?;
                Ok(format!("{}/", join(url, text).trim_end_matches('/')))
            }
        }
    }

    /// The calendar home, found through the principal.
    pub fn home(&self) -> Result<String, Refused> {
        let principal = self.find_principal()?;
        let body = "<propfind xmlns=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">\
                    <prop><c:calendar-home-set /></prop></propfind>";
        let reply = self.propfind(&principal, "0", body)?;
        if reply.status >= 400 {
            return Err(dav());
        }
        let doc = parse(utf8(&reply.body, "dav")?, "dav")?;
        let href = doc
            .root_element()
            .descendants()
            .filter(|n| is(n, C, "calendar-home-set"))
            .find_map(|n| kid(n, D, "href"))
            .and_then(|n| n.text())
            .filter(|t| !t.is_empty())
            .ok_or_else(dav)?;
        Ok(format!("{}/", join(&principal, href).trim_end_matches('/')))
    }

    /// The calendars at `url`, one level down.
    fn calendars_at(&self, url: &str) -> Result<Vec<String>, Soft> {
        let body = "<propfind xmlns=\"DAV:\"><prop><resourcetype /></prop></propfind>";
        let reply = self.propfind(url, "1", body).map_err(Soft::Hard)?;
        if reply.status >= 400 {
            return Err(Soft::Failed);
        }
        let text = utf8(&reply.body, "dav").map_err(|_| Soft::Failed)?;
        let doc = parse(text, "dav").map_err(|_| Soft::Failed)?;
        let mut done = HashSet::new();
        let mut found = Vec::new();
        for response in kids(doc.root_element(), D, "response") {
            let calendar = kids(response, D, "propstat")
                .filter_map(|ps| kid(ps, D, "prop"))
                .any(|prop| {
                    kids(prop, D, "resourcetype").any(|kind| kid(kind, C, "calendar").is_some())
                });
            if !calendar {
                continue;
            }
            let href = kid(response, D, "href").ok_or(Soft::Failed)?;
            let href = join(url, href.text().unwrap_or(""));
            if done.insert(href.clone()) {
                found.push(href);
            }
        }
        Ok(found)
    }

    /// The task lists on the server: the configured URL's calendars if it has any, else
    /// those in the calendar home. A list's id is the percent-decoded last path segment.
    /// Any id that is not plain is refused, as discovery does.
    pub fn list_home(&self) -> Result<Vec<Listed>, Refused> {
        let listed = self.list_home_raw()?;
        if listed.iter().any(|l| !plain(&l.id)) {
            return Err(Refused::new("mapping"));
        }
        Ok(listed)
    }

    /// [`Dav::list_home`] without judging the ids: every list with its decoded last path
    /// segment. A caller that picks one list validates that one only.
    pub fn list_home_raw(&self) -> Result<Vec<Listed>, Refused> {
        let hrefs = match self.calendars_at(&self.root) {
            Ok(found) if !found.is_empty() => found,
            Ok(_) | Err(Soft::Failed) => {
                let home = self.home()?;
                self.calendars_at(&home).map_err(Soft::refused)?
            }
            Err(Soft::Hard(refused)) => return Err(refused),
        };
        Ok(hrefs
            .into_iter()
            .map(|url| {
                let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                Listed {
                    id: unquote(last),
                    url,
                }
            })
            .collect())
    }

    /// One list, after checking that its URL names exactly `id` below the configured origin.
    pub fn collection(&self, url: &str, id: &str) -> Result<Collection<'_>, Refused> {
        validate_target(&self.configured, url, id)?;
        Ok(Collection {
            dav: self,
            url: format!("{}/", url.trim_end_matches('/')),
        })
    }
}

/// A failure the discovery lookups may recover from, as vdirsyncer's own do.
enum Soft {
    Failed,
    Hard(Refused),
}

impl Soft {
    fn refused(self) -> Refused {
        match self {
            Soft::Failed => dav(),
            Soft::Hard(refused) => refused,
        }
    }
}

// ---------------------------------------------------------------------------------------
// One collection
// ---------------------------------------------------------------------------------------

pub struct Collection<'a> {
    dav: &'a Dav,
    url: String,
}

impl Collection<'_> {
    fn request(
        &self,
        method: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Reply, Refused> {
        self.dav
            .send(method, &self.url, headers, body.map(str::as_bytes))
    }

    pub fn read_properties(&self) -> Result<Properties, Refused> {
        let body = "<d:propfind xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\" \
                    xmlns:a=\"http://apple.com/ns/ical/\">\
                    <d:prop><d:displayname/><d:resourcetype/><c:supported-calendar-component-set/>\
                    <a:calendar-color/></d:prop></d:propfind>";
        let reply = self.request(
            "PROPFIND",
            &[("Depth", "0"), ("Content-Type", XML_UTF8)],
            Some(body),
        )?;
        if reply.status != 207 {
            return Err(dav());
        }
        let doc = parse(utf8(&reply.body, "dav")?, "dav")?;
        let props = successful_properties(&doc, &self.url)?;
        let requested = [
            (D, "displayname"),
            (D, "resourcetype"),
            (C, "supported-calendar-component-set"),
            (A, "calendar-color"),
        ];
        let mut seen: HashSet<Tag> = HashSet::new();
        for propstat in kids(doc.root_element(), D, "response").flat_map(|r| kids(r, D, "propstat"))
        {
            let status = status_code(propstat);
            let prop = kid(propstat, D, "prop").ok_or_else(dav)?;
            for value in prop.children().filter(Node::is_element) {
                if !seen.insert(tag(value)) {
                    return Err(dav());
                }
                let optional = matches!(tag(value), (D, "displayname") | (A, "calendar-color"));
                if status != Some("200") && !(status == Some("404") && optional) {
                    return Err(dav());
                }
            }
        }
        if !requested.iter().all(|t| seen.contains(t)) {
            return Err(dav());
        }
        let calendar = props
            .get(&(D, "resourcetype"))
            .is_some_and(|kind| kid(*kind, C, "calendar").is_some());
        let vtodo = props
            .get(&(C, "supported-calendar-component-set"))
            .is_some_and(|set| {
                set.children()
                    .filter(Node::is_element)
                    .any(|c| c.attribute("name") == Some("VTODO"))
            });
        if !calendar || !vtodo {
            return Err(dav());
        }
        let text = |tag: Tag| props.get(&tag).and_then(|n| n.text()).map(str::to_string);
        Ok(Properties {
            name: text((D, "displayname")),
            color: text((A, "calendar-color")),
        })
    }

    /// Whether the list is gone from the server (a 404 for its properties).
    pub fn absent(&self) -> Result<bool, Refused> {
        let probe = "<d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/></d:prop></d:propfind>";
        let reply = self.dav.propfind(&self.url, "0", probe)?;
        match reply.status {
            404 => Ok(true),
            status if status >= 400 => Err(dav()),
            _ => Ok(false),
        }
    }

    pub fn create(&self, name: &str, color: &str) -> Result<Properties, Refused> {
        // Every failure up to the MKCALENDAR request, the probe included, is not sent.
        if !self.absent().map_err(Refused::not_sent)? {
            return Err(Refused::unsent("exists"));
        }
        let body = format!(
            "<c:mkcalendar xmlns:c=\"{C}\" xmlns:d=\"{D}\" xmlns:a=\"{A}\"><d:set><d:prop>\
             <d:displayname>{}</d:displayname><a:calendar-color>{}</a:calendar-color>\
             <c:supported-calendar-component-set><c:comp name=\"VTODO\"/>\
             </c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>",
            escape(name),
            escape(color)
        );
        let reply = self.request(
            "MKCALENDAR",
            &[("Content-Type", XML_UTF8), ("If-None-Match", "*")],
            Some(&body),
        )?;
        if reply.status != 201 {
            return Err(dav());
        }
        let after = self.read_properties()?;
        if after.name.as_deref() != Some(name)
            || after.color.as_deref().and_then(canonical_color) != canonical_color(color)
        {
            return Err(dav());
        }
        Ok(after)
    }

    /// Best effort: the server enforces no precondition, so the read-back is the only check.
    pub fn write_metadata(
        &self,
        changes: &Changes,
        expected: &Properties,
    ) -> Result<Properties, Refused> {
        if changes.name.is_none() && changes.color.is_none() {
            return Err(Refused::unsent("protocol"));
        }
        // Every failure up to the PROPPATCH request, the read included, is not sent.
        let before = self.read_properties().map_err(Refused::not_sent)?;
        let differs = |color: bool, now: &Option<String>, then: &Option<String>| {
            !same_metadata(color, now.as_deref(), then.as_deref())
        };
        if (changes.name.is_some() && differs(false, &before.name, &expected.name))
            || (changes.color.is_some() && differs(true, &before.color, &expected.color))
        {
            return Err(Refused::unsent("stale"));
        }
        let mut set = String::new();
        if let Some(name) = &changes.name {
            set.push_str(&format!("<d:displayname>{}</d:displayname>", escape(name)));
        }
        if let Some(color) = &changes.color {
            set.push_str(&format!(
                "<a:calendar-color>{}</a:calendar-color>",
                escape(color)
            ));
        }
        let body = format!(
            "<d:propertyupdate xmlns:d=\"{D}\" xmlns:a=\"{A}\"><d:set><d:prop>{set}</d:prop>\
             </d:set></d:propertyupdate>"
        );
        let reply = self.request("PROPPATCH", &[("Content-Type", XML_UTF8)], Some(&body))?;
        if reply.status != 207 {
            return Err(dav());
        }
        let doc = parse(utf8(&reply.body, "dav")?, "dav")?;
        let confirmed = successful_properties(&doc, &self.url)?;
        if (changes.name.is_some() && !confirmed.contains_key(&(D, "displayname")))
            || (changes.color.is_some() && !confirmed.contains_key(&(A, "calendar-color")))
        {
            return Err(dav());
        }
        let after = self.read_properties()?;
        let wrong = |color: bool, now: &Option<String>, wanted: &Option<String>| {
            wanted.is_some() && !same_metadata(color, now.as_deref(), wanted.as_deref())
        };
        if wrong(false, &after.name, &changes.name) || wrong(true, &after.color, &changes.color) {
            return Err(dav());
        }
        Ok(after)
    }

    fn raw_properties(&self) -> Result<Vec<u8>, Refused> {
        let reply = self.dav.propfind(
            &self.url,
            "0",
            "<d:propfind xmlns:d=\"DAV:\"><d:allprop/></d:propfind>",
        )?;
        if reply.status != 207 {
            return Err(Refused::new("inventory"));
        }
        let doc = parse(utf8(&reply.body, "inventory")?, "inventory")?;
        multistatus(&doc, true)?;
        successful_properties(&doc, &self.url)?;
        Ok(reply.body)
    }

    fn inventory(&self) -> Result<Vec<Entry>, Refused> {
        let inventory = || Refused::new("inventory");
        let reply = self.dav.propfind(
            &self.url,
            "1",
            "<d:propfind xmlns:d=\"DAV:\"><d:prop><d:getetag/><d:resourcetype/></d:prop>\
             </d:propfind>",
        )?;
        if reply.status != 207 {
            return Err(inventory());
        }
        let doc = parse(utf8(&reply.body, "inventory")?, "inventory")?;
        let base = format!("{}/", self.url.trim_end_matches('/'));
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let mut collections = 0;
        for row in multistatus(&doc, false)? {
            let href = findtext(row, D, "href")
                .filter(|h| !h.is_empty())
                .ok_or_else(inventory)?;
            let target = join(&base, href);
            if !seen.insert(target.clone()) {
                return Err(inventory());
            }
            let is_base = target.trim_end_matches('/') == base.trim_end_matches('/');
            let mut props: Props = HashMap::new();
            for propstat in kids(row, D, "propstat") {
                let prop = kid(propstat, D, "prop").ok_or_else(inventory)?;
                let status = status_code(propstat);
                if status != Some("200") {
                    let names: Vec<Tag> =
                        prop.children().filter(Node::is_element).map(tag).collect();
                    if is_base && status == Some("404") && names == [(D, "getetag")] {
                        continue;
                    }
                    return Err(inventory());
                }
                for value in prop.children().filter(Node::is_element) {
                    if props.insert(tag(value), value).is_some() {
                        return Err(inventory());
                    }
                }
            }
            let kind = props.get(&(D, "resourcetype"));
            if is_base {
                collections += 1;
                if !kind.is_some_and(|k| kid(*k, D, "collection").is_some()) {
                    return Err(inventory());
                }
                continue;
            }
            let name = target.strip_prefix(&base).unwrap_or("");
            let parts = split(&target);
            if !plain(&unquote(name))
                || name.contains('/')
                || !parts.query.is_empty()
                || !parts.fragment.is_empty()
            {
                return Err(inventory());
            }
            if !kind.is_some_and(|k| !k.children().any(|c| c.is_element())) {
                return Err(inventory());
            }
            let etag = props
                .get(&(D, "getetag"))
                .and_then(|n| n.text())
                .ok_or_else(inventory)?;
            if !strong_etag(etag) {
                return Err(inventory());
            }
            result.push(Entry {
                href: target,
                etag: etag.to_string(),
            });
        }
        if collections != 1 {
            return Err(inventory());
        }
        result.sort_by(|a, b| a.href.cmp(&b.href));
        Ok(result)
    }

    /// The list's raw properties and every resource, revalidated before returning.
    pub fn capture(&self) -> Result<Captured, Refused> {
        let inventory = || Refused::new("inventory");
        let properties = self.raw_properties()?;
        let before = self.inventory()?;
        let mut resources = Vec::new();
        let mut total = 0;
        for entry in &before {
            let reply = self
                .dav
                .send("GET", &entry.href, &[("If-Match", &entry.etag)], None)?;
            if reply.status != 200 || reply.etag.as_deref() != Some(&entry.etag) {
                return Err(inventory());
            }
            total += reply.body.len();
            if total > MAX_BACKUP {
                return Err(inventory());
            }
            resources.push(Resource {
                href: entry.href.clone(),
                etag: entry.etag.clone(),
                bytes: STANDARD.encode(&reply.body),
                sha256: hex(&Sha256::digest(&reply.body)),
            });
        }
        if before != self.inventory()? || properties != self.raw_properties()? {
            return Err(Refused::unsent("stale"));
        }
        Ok(Captured {
            format: 1,
            properties: STANDARD.encode(&properties),
            resources,
        })
    }

    /// Best effort: nothing on the server enforces the revalidated state at DELETE time.
    pub fn delete(&self, backup: &Captured) -> Result<(), Refused> {
        // Every failure up to here happens before the DELETE is sent.
        let unchanged = (|| {
            self.read_properties()?;
            backup_matches(backup, &self.capture()?)
        })()
        .map_err(Refused::not_sent)?;
        if !unchanged {
            return Err(Refused::unsent("changed"));
        }
        let reply = self.request("DELETE", &[], None)?;
        match reply.status {
            200 | 202 | 204 | 404 => Ok(()),
            _ => Err(dav()),
        }
    }

    /// Read-only: never sends DELETE. `Ok` means the list is gone.
    pub fn reconcile_delete(&self, backup: &Captured) -> Result<(), Refused> {
        if self.absent()? {
            return Ok(());
        }
        self.read_properties()?;
        let captured = self.capture()?;
        if backup_matches(backup, &captured)? {
            return Err(Refused::new("present"));
        }
        Err(Refused::new(if partly_removed(backup, &captured) {
            "partial"
        } else {
            "changed"
        }))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A quoted, non-weak ETag without spaces or control characters.
fn strong_etag(etag: &str) -> bool {
    etag.len() >= 3
        && etag.starts_with('"')
        && etag.ends_with('"')
        && !etag[1..etag.len() - 1]
            .chars()
            .any(|c| c == '"' || c <= ' ' || c == '\x7f')
}

type Identity<'a> = (&'a str, &'a str, &'a str);

fn identities(resources: &[Resource]) -> HashSet<Identity<'_>> {
    resources
        .iter()
        .map(|r| (r.href.as_str(), r.etag.as_str(), r.sha256.as_str()))
        .collect()
}

/// Properties by decoded bytes, resources as the set of (href, etag, sha256).
fn backup_matches(backup: &Captured, captured: &Captured) -> Result<bool, Refused> {
    let operation = || Refused::new("operation");
    let decode = |text: &str| STANDARD.decode(text).map_err(|_| operation());
    if decode(&backup.properties)? != decode(&captured.properties)? {
        return Ok(false);
    }
    let wanted = identities(&backup.resources);
    if wanted.len() != backup.resources.len() {
        return Err(operation());
    }
    Ok(wanted == identities(&captured.resources))
}

/// Some backed-up resources are gone and nothing else changed: a partly applied DELETE.
fn partly_removed(backup: &Captured, captured: &Captured) -> bool {
    let wanted = identities(&backup.resources);
    let now = identities(&captured.resources);
    now.len() < wanted.len() && now.is_subset(&wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_href_keeps_its_dot_segments_for_the_target_check() {
        let base = "https://dav.example.org/dav/alice/";
        let joined = join(base, "https://dav.example.org/dav/alice/../work/");
        assert_eq!(joined, "https://dav.example.org/dav/alice/../work/");
        assert_eq!(
            validate_target(base, &joined, "work").unwrap_err().code,
            "mapping"
        );
        let network = join(base, "//dav.example.org/dav/alice/../work/");
        assert_eq!(
            validate_target(base, &network, "work").unwrap_err().code,
            "mapping"
        );
        // Relative references are still resolved.
        assert_eq!(
            join(base, "../bob/work/"),
            "https://dav.example.org/dav/bob/work/"
        );
    }
}
