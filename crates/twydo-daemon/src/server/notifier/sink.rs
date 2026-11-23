//! Notification sinks: how a delivery reaches the desktop.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use twydo_core::remind::Delivery;

pub trait Sink: Send + Sync {
    fn deliver(&self, d: &Delivery) -> Result<(), String>;
}

pub const TIMEOUT: Duration = Duration::from_secs(10);

/// `notify-send` on Linux; on macOS `terminal-notifier` when on PATH, else `osascript`.
pub struct PlatformSink;

/// `argv` then title and body as the last two arguments.
pub struct CommandSink {
    pub argv: Vec<String>,
    pub timeout: Duration,
}

/// The markup escape the Linux sink applies to a body (the daemon scrubs it from errors).
pub fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn linux_args(d: &Delivery) -> Vec<String> {
    let body = escape_markup(&d.body);
    vec![
        "--app-name=Twydo".into(),
        "--".into(),
        d.title.clone(),
        body,
    ]
}

/// The tool's own escape: a leading `-` or `[` would be read as an option or a list.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn tn_escape(s: &str) -> String {
    if s.starts_with('-') || s.starts_with('[') {
        format!("\\{s}")
    } else {
        s.to_string()
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn terminal_notifier_args(d: &Delivery) -> Vec<String> {
    vec![
        "-title".into(),
        tn_escape(&d.title),
        "-message".into(),
        tn_escape(&d.body),
        "-group".into(),
        d.key.clone(),
    ]
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn osascript_args(d: &Delivery) -> Vec<String> {
    let mut a: Vec<String> = [
        "-e",
        "on run argv",
        "-e",
        "display notification (item 2 of argv) with title (item 1 of argv)",
        "-e",
        "end run",
        "--",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(d.title.clone());
    a.push(d.body.clone());
    a
}

#[cfg(target_os = "macos")]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
        .unwrap_or(false)
}

impl Sink for PlatformSink {
    #[cfg(target_os = "macos")]
    fn deliver(&self, d: &Delivery) -> Result<(), String> {
        if on_path("terminal-notifier") {
            run(
                "terminal-notifier",
                &terminal_notifier_args(d),
                &[],
                TIMEOUT,
            )
        } else {
            run("osascript", &osascript_args(d), &[], TIMEOUT)
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn deliver(&self, d: &Delivery) -> Result<(), String> {
        run("notify-send", &linux_args(d), &[], TIMEOUT)
    }
}

impl Sink for CommandSink {
    fn deliver(&self, d: &Delivery) -> Result<(), String> {
        let (program, rest) = self
            .argv
            .split_first()
            .ok_or_else(|| "empty notification command".to_string())?;
        let mut args = rest.to_vec();
        args.push(d.title.clone());
        args.push(d.body.clone());
        let open = d.today_open.map(|n| n.to_string());
        let envs = [
            ("TWYDO_REMINDER_KEY", Some(d.key.as_str())),
            ("TWYDO_REMINDER_UID", d.uid.as_deref()),
            ("TWYDO_TODAY_OPEN", open.as_deref()),
        ];
        run(program, &args, &envs, self.timeout)
    }
}

/// Runs `program args` without a shell, stdin/stdout null, in its own process group; kills
/// the group at `timeout`. Err: cannot start, non-zero exit (with the first stderr line), or
/// timed out. `envs` entries set (`Some`) or remove (`None`) a variable in the child.
pub fn run(
    program: &str,
    args: &[String],
    envs: &[(&str, Option<&str>)],
    timeout: Duration,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    for (k, v) in envs {
        match v {
            Some(v) => command.env(k, v),
            None => command.env_remove(k),
        };
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    let pipe = child.stderr.take();
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let Some(p) = pipe else { return };
        let mut p = std::io::BufReader::new(p);
        let mut line = String::new();
        let _ = std::io::BufRead::read_line(&mut (&mut p).take(8192), &mut line);
        let _ = line_tx.send(line);
        // Keep draining so the child never blocks on a full pipe.
        let _ = std::io::copy(&mut p, &mut std::io::sink());
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(e) => return Err(format!("{program}: {e}")),
        }
        if Instant::now() >= deadline {
            // SAFETY: killpg of the child's own group (spawned with process_group(0)).
            unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
            let _ = child.kill();
            let _ = child.wait();
            // Not joined: a descendant outside the group may still hold the pipe.
            return Err(format!(
                "{program}: timed out after {} s",
                timeout.as_secs_f64().ceil() as u64
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if status.success() {
        return Ok(());
    }
    // A descendant may hold the pipe open past the leader's exit: wait a short, bounded
    // grace for the first line instead of for the pipe to close.
    let stderr = line_rx
        .recv_timeout(Duration::from_millis(200))
        .unwrap_or_default();
    let first = stderr.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        Err(format!("{program} failed: {status}"))
    } else {
        Err(format!("{program} failed: {status}: {first}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(title: &str, body: &str) -> Delivery {
        Delivery {
            key: "key-1".into(),
            uid: Some("uid-1".into()),
            title: title.into(),
            body: body.into(),
            today_open: None,
        }
    }

    #[test]
    fn linux_args_put_text_after_double_dash() {
        let a = linux_args(&d("-rf", "body"));
        assert_eq!(a, ["--app-name=Twydo", "--", "-rf", "body"]);
    }

    #[test]
    fn linux_body_is_escaped() {
        let a = linux_args(&d("a <b>", "a <b> & c"));
        assert_eq!(a[2], "a <b>");
        assert_eq!(a[3], "a &lt;b&gt; &amp; c");
    }

    #[test]
    fn terminal_notifier_args() {
        let a = super::terminal_notifier_args(&d("[x]", "-y"));
        assert_eq!(
            a,
            ["-title", "\\[x]", "-message", "\\-y", "-group", "key-1"]
        );
    }

    #[test]
    fn osascript_args_never_contain_task_text_in_script() {
        let a = osascript_args(&d("on run", "end run"));
        let fixed = [
            "on run argv",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "end run",
        ];
        let mut i = 0;
        while i < a.len() {
            if a[i] == "-e" {
                assert!(fixed.contains(&a[i + 1].as_str()));
                i += 1;
            }
            i += 1;
        }
        assert_eq!(a.iter().filter(|s| *s == "-e").count(), 3);
        assert_eq!(a[a.len() - 2..], ["on run", "end run"]);
    }

    #[test]
    fn command_sink_appends_title_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let script = format!("printf '%s\\n' \"$@\" > '{}'", out.display());
        let sink = CommandSink {
            argv: vec!["sh".into(), "-c".into(), script, "sh".into()],
            timeout: TIMEOUT,
        };
        sink.deliver(&d("the title", "the body")).unwrap();
        assert_eq!(
            std::fs::read_to_string(out).unwrap(),
            "the title\nthe body\n"
        );
    }

    #[test]
    fn run_reports_non_zero_exit() {
        let e = run(
            "sh",
            &["-c".into(), "echo boom >&2; exit 3".into()],
            &[],
            TIMEOUT,
        )
        .unwrap_err();
        assert!(e.contains("boom"), "{e}");
    }

    #[test]
    fn run_returns_when_a_descendant_keeps_stderr_open() {
        let t = Instant::now();
        let e = run(
            "sh",
            &["-c".into(), "sleep 30 >&2 & echo boom >&2; exit 3".into()],
            &[],
            Duration::from_millis(500),
        )
        .unwrap_err();
        assert!(e.contains("boom"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn run_reports_missing_program() {
        let e = run("twydo-no-such-program", &[], &[], TIMEOUT).unwrap_err();
        assert!(e.contains("cannot start"), "{e}");
    }

    #[test]
    fn run_kills_a_hung_command() {
        let t = Instant::now();
        let e = run("sleep", &["30".into()], &[], Duration::from_millis(200)).unwrap_err();
        assert!(e.contains("timed out"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
