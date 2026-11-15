//! A fake CalDAV server and a fake `vdirsyncer` script for Twydo's tests.
//!
//! All state lives in files under a root directory, so a daemon in another process and the
//! shell script see what the server sees:
//!
//! - `server.json`: `{"collections": {id: {"name", "color", "resources": {name: {"etag",
//!   "body"}}}}}`, rewritten atomically.
//! - `collections.txt`: the collection ids, one per line, for the shell script.
//! - `flags/<name>`: presence means set, the content is the value. `_once` flags are deleted
//!   when used.
//! - `slow_write` (a flag with the seconds as its value): a PROPPATCH sleeps that long, after
//!   it is logged and before it is applied.
//! - `calls.log`: one line per authenticated request method, plus `DISCOVER`, `SYNC` and
//!   `METASYNC` from the script.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};

const BASIC_ALICE: &str = "Basic YWxpY2U6c2VjcmV0";
const HOME: &str = "/dav/alice/";
const PRINCIPAL: &str = "/dav/principals/alice/";
const OVERSIZE: usize = 5 * 1024 * 1024;

// ---------------------------------------------------------------------------------------
// State files
// ---------------------------------------------------------------------------------------

fn read_state(root: &Path) -> Value {
    fs::read_to_string(root.join("server.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({"collections": {}}))
}

fn replace_file(path: &Path, content: &str) {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let unique = WRITES.fetch_add(1, Ordering::SeqCst);
    let tmp = path.with_extension(format!("tmp{}-{unique}", std::process::id()));
    fs::write(&tmp, content).expect("write state file");
    fs::rename(&tmp, path).expect("replace state file");
}

fn write_state(root: &Path, state: &Value) {
    fs::create_dir_all(root).expect("create root");
    let ids: String = state["collections"]
        .as_object()
        .map(|c| c.keys().map(|id| format!("{id}\n")).collect())
        .unwrap_or_default();
    replace_file(&root.join("collections.txt"), &ids);
    replace_file(&root.join("server.json"), &state.to_string());
}

fn flag_get(root: &Path, name: &str) -> Option<String> {
    fs::read_to_string(root.join("flags").join(name))
        .ok()
        .map(|v| v.trim().to_string())
}

fn flag_take(root: &Path, name: &str) -> Option<String> {
    let value = flag_get(root, name)?;
    let _ = fs::remove_file(root.join("flags").join(name));
    Some(value)
}

fn log_call(root: &Path, name: &str) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("calls.log"))
        .expect("open calls.log");
    file.write_all(format!("{name}\n").as_bytes())
        .expect("append call");
}

/// Adds (or replaces) a collection with no resources.
pub fn add_collection(root: &Path, id: &str, name: &str, color: &str) {
    let mut state = read_state(root);
    state["collections"][id] = json!({"name": name, "color": color, "resources": {}});
    write_state(root, &state);
}

/// Removes a collection, as if it had been deleted from the server by another device.
pub fn remove_collection(root: &Path, id: &str) {
    let mut state = read_state(root);
    if let Some(all) = state["collections"].as_object_mut() {
        all.remove(id);
    }
    write_state(root, &state);
}

/// Adds a resource to a collection. The etag is stored as given, quotes included.
pub fn add_resource(root: &Path, id: &str, name: &str, etag: &str, body: &str) {
    let mut state = read_state(root);
    state["collections"][id]["resources"][name] = json!({"etag": etag, "body": body});
    write_state(root, &state);
}

/// Every collection as `id -> (name, color)`.
pub fn collections(root: &Path) -> BTreeMap<String, (Option<String>, Option<String>)> {
    let state = read_state(root);
    let text = |v: &Value| v.as_str().map(str::to_string);
    state["collections"]
        .as_object()
        .map(|all| {
            all.iter()
                .map(|(id, c)| (id.clone(), (text(&c["name"]), text(&c["color"]))))
                .collect()
        })
        .unwrap_or_default()
}

/// Sets a flag with an empty value.
pub fn flag(root: &Path, name: &str) {
    flag_value(root, name, "");
}

/// Sets a flag with a value.
pub fn flag_value(root: &Path, name: &str, value: &str) {
    let dir = root.join("flags");
    fs::create_dir_all(&dir).expect("create flags dir");
    fs::write(dir.join(name), value).expect("write flag");
}

/// The request methods and script subcommands logged so far, in order.
pub fn calls(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("calls.log"))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------
// The fake vdirsyncer
// ---------------------------------------------------------------------------------------

/// Writes an executable `sh` script that logs its subcommand and answers `discover` from the
/// server's collection list. It needs `TWYDO_SYNC_VDIR` and uses `VDIRSYNCER_CONFIG`.
pub fn fake_vdirsyncer(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create root");
    let quoted = root.to_string_lossy().replace('\'', "'\\''");
    let script = format!(
        r#"#!/bin/sh
ROOT='{quoted}'
case "$1" in
  discover) echo DISCOVER >> "$ROOT/calls.log" ;;
  sync) echo SYNC >> "$ROOT/calls.log" ;;
  metasync) echo METASYNC >> "$ROOT/calls.log" ;;
esac
[ "$1" = discover ] || exit 0
if [ -f "$ROOT/flags/fail_refresh_once" ]; then
  rm -f "$ROOT/flags/fail_refresh_once"
  echo "error: synthetic interrupted discovery" >&2
  exit 1
fi
[ -f "$ROOT/flags/discover_sleep" ] && sleep "$(cat "$ROOT/flags/discover_sleep")"
[ -f "$ROOT/flags/hang_after_adopt" ] && sleep "$(cat "$ROOT/flags/hang_after_adopt")"
V="$TWYDO_SYNC_VDIR"
missing=
if [ -f "$ROOT/collections.txt" ]; then
  while IFS= read -r id; do
    [ -z "$id" ] && continue
    [ -d "$V/$id" ] || missing=1
  done < "$ROOT/collections.txt"
fi
for d in "$V"/*/; do
  [ -d "$d" ] || continue
  grep -qxF -- "$(basename "$d")" "$ROOT/collections.txt" 2>/dev/null || missing=1
done
if [ -n "$missing" ]; then
  echo "error: Unable to find or create collection" >&2
  exit 1
fi
status="$(dirname "${{VDIRSYNCER_CONFIG:-./config}}")/status"
mkdir -p "$status"
echo "verified synthetic discovery" > "$status/twydo.collections"
exit 0
"#
    );
    let path = root.join("vdirsyncer");
    // Rewriting a script another thread is about to run can fail with ETXTBSY.
    if fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        let tmp = root.join("vdirsyncer.tmp");
        fs::write(&tmp, &script).expect("write script");
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).expect("chmod script");
        fs::rename(&tmp, &path).expect("install script");
    }
    path
}

// ---------------------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------------------

/// The fake server. Dropping it stops it.
pub struct FakeDav {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeDav {
    /// Starts serving on a free local port, with its state under `root`.
    pub fn start(root: &Path) -> FakeDav {
        fs::create_dir_all(root).expect("create root");
        if !root.join("server.json").exists() {
            write_state(root, &json!({"collections": {}}));
        }
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let stop = Arc::new(AtomicBool::new(false));
        let server = Server {
            root: root.to_path_buf(),
            stop: stop.clone(),
            inventories: Arc::default(),
        };
        let handle = thread::spawn(move || server.run(listener));
        FakeDav {
            port,
            stop,
            handle: Some(handle),
        }
    }

    /// The value for `[sync].url`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}{HOME}", self.port)
    }
}

impl Drop for FakeDav {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

enum Reply {
    Response {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    },
    /// Close the connection without answering: a transport failure.
    Drop,
}

fn reply(status: u16, body: impl Into<Vec<u8>>) -> Reply {
    Reply::Response {
        status,
        headers: Vec::new(),
        body: body.into(),
    }
}

struct Resource {
    name: String,
    etag: String,
    body: String,
}

#[derive(Clone)]
struct Server {
    root: PathBuf,
    stop: Arc<AtomicBool>,
    /// Depth-1 listings of a collection since its last named property read: the `changed`
    /// fault alters the listing from the second one on.
    inventories: Arc<Mutex<HashMap<String, u32>>>,
}

impl Server {
    fn run(self, listener: TcpListener) {
        for stream in listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            // A request that hangs must not block the others.
            let mut server = self.clone();
            thread::spawn(move || server.serve(stream));
        }
    }

    fn serve(&mut self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let Some(request) = read_request(&mut stream) else {
            return;
        };
        if let Reply::Response {
            status,
            headers,
            body,
        } = self.handle(&request)
        {
            let mut head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Type: application/xml; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n",
                reason(status),
                body.len()
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
        // Otherwise the stream is dropped unanswered.
    }

    fn sleep(&self, seconds: f64) {
        let mut left = Duration::from_secs_f64(seconds);
        let step = Duration::from_millis(50);
        while !left.is_zero() && !self.stop.load(Ordering::SeqCst) {
            let now = left.min(step);
            thread::sleep(now);
            left -= now;
        }
    }

    fn hang_seconds(&self) -> f64 {
        flag_get(&self.root, "hang_seconds")
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0)
    }

    fn handle(&mut self, request: &Request) -> Reply {
        let root = self.root.clone();
        if request.headers.get("authorization").map(String::as_str) != Some(BASIC_ALICE) {
            return Reply::Response {
                status: 401,
                headers: vec![("WWW-Authenticate", "Basic realm=\"fake\"".to_string())],
                body: Vec::new(),
            };
        }
        log_call(&root, &request.method);
        if flag_get(&root, "redirect").is_some() {
            return Reply::Response {
                status: 302,
                headers: vec![("Location", "http://127.0.0.1:1/".to_string())],
                body: Vec::new(),
            };
        }
        let mut answer = self.route(request);
        if flag_get(&root, "oversize").is_some()
            && let Reply::Response { body, .. } = &mut answer
            && body.len() < OVERSIZE
        {
            body.resize(OVERSIZE, b' ');
        }
        answer
    }

    fn route(&mut self, request: &Request) -> Reply {
        let path = request.path.split('?').next().unwrap_or("");
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let depth = request
            .headers
            .get("depth")
            .map(String::as_str)
            .unwrap_or("0");
        let method = request.method.as_str();
        match parts.as_slice() {
            ["dav", "alice"] if method == "PROPFIND" => {
                if depth == "1" {
                    self.home_listing()
                } else {
                    multistatus(&format!(
                        "<d:response><d:href>{HOME}</d:href>{}</d:response>",
                        propstat(
                            "200 OK",
                            &format!(
                                "<d:current-user-principal><d:href>{PRINCIPAL}</d:href>\
                                 </d:current-user-principal>"
                            )
                        )
                    ))
                }
            }
            ["dav", "principals", "alice"] if method == "PROPFIND" => multistatus(&format!(
                "<d:response><d:href>{PRINCIPAL}</d:href>{}</d:response>",
                propstat(
                    "200 OK",
                    &format!("<c:calendar-home-set><d:href>{HOME}</d:href></c:calendar-home-set>")
                )
            )),
            ["dav", "alice", id] => self.collection(request, id, depth),
            ["dav", "alice", id, name] if method == "GET" => self.resource(request, id, name),
            _ => reply(404, ""),
        }
    }

    fn home_listing(&self) -> Reply {
        let state = read_state(&self.root);
        let mut rows = format!(
            "<d:response><d:href>{HOME}</d:href>{}</d:response>",
            propstat("200 OK", "<d:resourcetype><d:collection/></d:resourcetype>")
        );
        for (id, c) in state["collections"].as_object().into_iter().flatten() {
            rows.push_str(&format!(
                "<d:response><d:href>{HOME}{id}/</d:href>{}</d:response>",
                propstat("200 OK", &collection_props(c))
            ));
        }
        multistatus(&rows)
    }

    fn resources(&self, collection: &Value) -> Vec<Resource> {
        collection["resources"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, r)| Resource {
                name: name.clone(),
                etag: r["etag"].as_str().unwrap_or("\"v\"").to_string(),
                body: r["body"].as_str().unwrap_or("").to_string(),
            })
            .collect()
    }

    fn inventory_fault(&self) -> Option<String> {
        flag_get(&self.root, "inventory_fault")
            .or_else(|| flag_get(&self.root, "mutate_before_delete").map(|_| "extra".to_string()))
    }

    /// The etag a listing or GET reports for a resource under the current fault.
    fn etag_for(&self, fault: &Option<String>, etag: &str, inventories: u32) -> String {
        match fault.as_deref() {
            Some("etag-changed") => format!("{}-new\"", etag.trim_end_matches('"')),
            Some("weak") => format!("W/{etag}"),
            Some("changed") if inventories > 1 => "\"changed\"".to_string(),
            _ => etag.to_string(),
        }
    }

    fn collection(&mut self, request: &Request, id: &str, depth: &str) -> Reply {
        let root = self.root.clone();
        let mut state = read_state(&root);
        let exists = state["collections"].get(id).is_some();
        match request.method.as_str() {
            "MKCALENDAR" => {
                if exists {
                    return reply(405, "");
                }
                let name = xml_text(&request.body, "displayname");
                let color = xml_text(&request.body, "calendar-color");
                state["collections"][id] = json!({"name": name, "color": color, "resources": {}});
                write_state(&root, &state);
                if flag_take(&root, "hang_after_create_once").is_some() {
                    self.sleep(self.hang_seconds());
                }
                reply(201, "")
            }
            _ if !exists => reply(404, ""),
            "DELETE" => {
                if flag_take(&root, "fail_delete_once").is_some() {
                    return Reply::Drop;
                }
                if flag_take(&root, "partial_delete_once").is_some() {
                    if let Some(resources) = state["collections"][id]["resources"].as_object_mut()
                        && let Some(first) = resources.keys().next().cloned()
                    {
                        resources.remove(&first);
                    }
                    write_state(&root, &state);
                    return Reply::Drop;
                }
                state["collections"].as_object_mut().unwrap().remove(id);
                write_state(&root, &state);
                if flag_take(&root, "hang_after_delete_once").is_some() {
                    self.sleep(self.hang_seconds());
                }
                reply(204, "")
            }
            "PROPPATCH" => {
                if let Some(seconds) = flag_get(&root, "slow_write") {
                    self.sleep(seconds.parse().unwrap_or(1.0));
                }
                let fail = flag_get(&root, "fail_status").is_some();
                let ignore = flag_get(&root, "ignore_write").is_some();
                let mut echoed = String::new();
                for (local, field, tag) in [
                    ("displayname", "name", "d:displayname"),
                    ("calendar-color", "color", "a:calendar-color"),
                ] {
                    if let Some(value) = xml_text(&request.body, local) {
                        echoed.push_str(&format!("<{tag}/>"));
                        if !ignore && !fail {
                            state["collections"][id][field] = Value::String(value);
                        }
                    }
                }
                write_state(&root, &state);
                let status = if fail { "403 Forbidden" } else { "200 OK" };
                multistatus(&format!(
                    "<d:response><d:href>{HOME}{id}/</d:href>{}</d:response>",
                    propstat(status, &echoed)
                ))
            }
            "PROPFIND" if depth == "1" => self.inventory(&state, id),
            "PROPFIND" => {
                let named = request.body.contains("displayname");
                if named {
                    self.inventories.lock().unwrap().remove(id);
                }
                let fault = self.inventory_fault();
                let mut collection = state["collections"][id].clone();
                if fault.as_deref() == Some("props-changed") {
                    collection["name"] = Value::String("Renamed".to_string());
                }
                let mut ok = propstat("200 OK", &collection_props(&collection));
                if !request.body.contains("allprop") {
                    let mut absent = String::new();
                    if collection["name"].is_null() {
                        absent.push_str("<d:displayname/>");
                    }
                    if collection["color"].is_null() {
                        absent.push_str("<a:calendar-color/>");
                    }
                    if !absent.is_empty() {
                        ok.push_str(&propstat("404 Not Found", &absent));
                    }
                }
                if fault.as_deref() == Some("properties") {
                    ok = ok.replace("200 OK", "403 Forbidden");
                }
                multistatus(&format!(
                    "<d:response><d:href>{HOME}{id}/</d:href>{ok}</d:response>"
                ))
            }
            _ => reply(405, ""),
        }
    }

    fn inventory(&mut self, state: &Value, id: &str) -> Reply {
        let count = {
            let mut all = self.inventories.lock().unwrap();
            let n = all.entry(id.to_string()).or_insert(0);
            *n += 1;
            *n
        };
        let fault = self.inventory_fault();
        let collection = &state["collections"][id];
        let mut resources = self.resources(collection);
        match fault.as_deref() {
            Some("extra") => resources.push(Resource {
                name: "added.ics".into(),
                etag: "\"added\"".into(),
                body: "added synthetic resource".into(),
            }),
            Some("partial") if !resources.is_empty() => {
                resources.remove(0);
            }
            _ => {}
        }
        let failed = fault.as_deref() == Some("failed");
        let status = if failed { "403 Forbidden" } else { "200 OK" };
        let base = format!("{HOME}{id}/");
        let mut rows = String::new();
        let mut collection_row = format!(
            "<d:response><d:href>{base}</d:href>{}",
            propstat(status, "<d:resourcetype><d:collection/></d:resourcetype>")
        );
        if fault.as_deref() == Some("collection-etag-absent") {
            collection_row.push_str(&propstat("404 Not Found", "<d:getetag/>"));
        }
        collection_row.push_str("</d:response>");
        rows.push_str(&collection_row);
        for r in &resources {
            let href = match fault.as_deref() {
                Some("foreign") => format!("https://elsewhere.example.org/{}", r.name),
                Some("nested") => format!("{base}nested/{}", r.name),
                _ => format!("{base}{}", r.name),
            };
            let mut props = "<d:resourcetype/>".to_string();
            if fault.as_deref() != Some("missing") {
                props.push_str(&format!(
                    "<d:getetag>{}</d:getetag>",
                    escape(&self.etag_for(&fault, &r.etag, count))
                ));
            }
            let row = format!(
                "<d:response><d:href>{}</d:href>{}</d:response>",
                escape(&href),
                propstat(status, &props)
            );
            rows.push_str(&row);
            if fault.as_deref() == Some("duplicate") {
                rows.push_str(&row);
            }
        }
        multistatus(&rows)
    }

    fn resource(&mut self, request: &Request, id: &str, name: &str) -> Reply {
        let state = read_state(&self.root);
        let Some(collection) = state["collections"].get(id) else {
            return reply(404, "");
        };
        let fault = self.inventory_fault();
        let mut resources = self.resources(collection);
        if fault.as_deref() == Some("extra") {
            resources.push(Resource {
                name: "added.ics".into(),
                etag: "\"added\"".into(),
                body: "added synthetic resource".into(),
            });
        }
        let Some(position) = resources.iter().position(|r| r.name == name) else {
            return reply(404, "");
        };
        let found = &resources[position];
        let inventories = self
            .inventories
            .lock()
            .unwrap()
            .get(id)
            .copied()
            .unwrap_or(0);
        let current = self.etag_for(&fault, &found.etag, inventories);
        if let Some(wanted) = request.headers.get("if-match")
            && *wanted != current
        {
            return reply(412, "");
        }
        let mut body = found.body.clone().into_bytes();
        if fault.as_deref() == Some("bytes-changed") && position == 0 {
            body.push(b'!');
        }
        let reported = if fault.as_deref() == Some("get-stale") {
            "\"changed\"".to_string()
        } else {
            current
        };
        Reply::Response {
            status: 200,
            headers: vec![("ETag", reported)],
            body,
        }
    }
}

// ---------------------------------------------------------------------------------------
// HTTP and XML helpers
// ---------------------------------------------------------------------------------------

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        207 => "Multi-Status",
        302 => "Found",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        412 => "Precondition Failed",
        _ => "Status",
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let body_start = head_end + 4;
    while data.len() < body_start + length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&data[body_start..body_start + length]).into_owned();
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text of the first element whose local name is `local`, whatever its prefix.
/// `Some("")` for an empty element; `None` when it does not occur.
fn xml_text(xml: &str, local: &str) -> Option<String> {
    let mut from = 0;
    while let Some(open) = xml[from..].find('<').map(|i| i + from) {
        let tag_end = xml[open..].find('>').map(|i| i + open)?;
        let tag = &xml[open + 1..tag_end];
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        if !name.starts_with('/') && name.rsplit(':').next() == Some(local) {
            if tag.ends_with('/') {
                return Some(String::new());
            }
            let end = xml[tag_end + 1..].find('<').map(|i| i + tag_end + 1)?;
            return Some(unescape(&xml[tag_end + 1..end]));
        }
        from = tag_end + 1;
    }
    None
}

fn propstat(status: &str, props: &str) -> String {
    format!(
        "<d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 {status}</d:status></d:propstat>"
    )
}

fn multistatus(rows: &str) -> Reply {
    reply(
        207,
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:multistatus xmlns:d=\"DAV:\" \
             xmlns:c=\"urn:ietf:params:xml:ns:caldav\" \
             xmlns:a=\"http://apple.com/ns/ical/\">{rows}</d:multistatus>"
        ),
    )
}

/// The properties a collection has; absent ones are left out, never emitted empty.
fn collection_props(collection: &Value) -> String {
    let mut props = String::new();
    if let Some(name) = collection["name"].as_str() {
        props.push_str(&format!("<d:displayname>{}</d:displayname>", escape(name)));
    }
    if let Some(color) = collection["color"].as_str() {
        props.push_str(&format!(
            "<a:calendar-color>{}</a:calendar-color>",
            escape(color)
        ));
    }
    props.push_str(
        "<d:resourcetype><d:collection/><c:calendar/></d:resourcetype>\
         <c:supported-calendar-component-set><c:comp name=\"VTODO\"/>\
         </c:supported-calendar-component-set>",
    );
    props
}
