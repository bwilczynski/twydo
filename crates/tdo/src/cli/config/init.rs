//! `tdo config init`: guided setup of `config.toml`. Never contacts the daemon.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::Args;
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};
use twydo_core::config::expand_tilde;

use super::prompt::Prompter;
use super::write::{home_relative, write_through};

const DEFAULT_VDIR: &str = "~/.local/share/twydo/vdir";
const Q_VDIR: &str = "Vdir";
const Q_SYNC: &str = "Sync with a CalDAV server?";
const Q_URL: &str = "Server URL";
const Q_USER: &str = "Username";
const Q_PASS: &str = "Password command (prints the password, e.g. pass show twydo/caldav)";
/// Keys the generated vdirsyncer config replaced.
const REMOVED: [&str; 4] = ["pair", "local_storage", "vdirsyncer_config", "after_edit"];

#[derive(Args, Debug, Default)]
pub struct InitArgs {
    /// Do not set up sync (writes no `[sync]` table)
    #[arg(long)]
    pub no_sync: bool,
    /// CalDAV server URL
    #[arg(long, conflicts_with = "no_sync")]
    pub url: Option<String>,
    /// CalDAV username
    #[arg(long, conflicts_with = "no_sync")]
    pub username: Option<String>,
    /// Command that prints the password, as one string split like a shell line
    #[arg(long, value_name = "CMD", conflicts_with = "no_sync")]
    pub password_command: Option<String>,
    /// CA certificate (PEM) for a server whose certificate is not trusted
    #[arg(long, value_name = "PATH")]
    pub ca_file: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("cancelled; nothing was written")]
    Cancelled,
    #[error("{flag} is required when not running interactively")]
    Required { flag: &'static str },
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("existing config is not valid TOML: {0}")]
    Toml(#[from] toml_edit::TomlError),
    #[error("--password-command: {0}")]
    BadCommand(String),
}

/// What the existing file says, read leniently (any keys allowed).
#[derive(Default)]
struct Existing {
    vdir: Option<String>,
    has_sync: bool,
    url: Option<String>,
    username: Option<String>,
    password_command: Option<String>,
}

fn existing_values(doc: &DocumentMut) -> Existing {
    let str_of = |t: &dyn TableLike, k: &str| t.get(k).and_then(Item::as_str).map(String::from);
    let mut e = Existing {
        vdir: str_of(doc.as_table(), "vdir"),
        ..Existing::default()
    };
    if let Some(sync) = doc.get("sync").and_then(Item::as_table_like) {
        e.has_sync = true;
        e.url = str_of(sync, "url");
        e.username = str_of(sync, "username");
        e.password_command = sync
            .get("password_command")
            .and_then(Item::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .filter(|v| !v.is_empty())
            .map(shell_words::join);
    }
    e
}

/// Ask until the answer is non-empty (Enter takes `default`); `ok` may reject an answer.
fn ask(
    p: &mut dyn Prompter,
    question: &str,
    default: Option<&str>,
    ok: impl Fn(&str) -> Result<(), String>,
) -> Result<String, InitError> {
    loop {
        let Some(a) = p.line(question, default)? else {
            return Err(InitError::Cancelled);
        };
        let a = if a.is_empty() {
            match default {
                Some(d) => d.to_string(),
                None => continue,
            }
        } else {
            a
        };
        match ok(&a) {
            Ok(()) => return Ok(a),
            Err(m) => eprintln!("{m}"),
        }
    }
}

fn split_command(s: &str) -> Result<Vec<String>, String> {
    match shell_words::split(s) {
        Ok(v) if v.is_empty() => Err("the command is empty".into()),
        Ok(v) => Ok(v),
        Err(e) => Err(format!("cannot parse the command: {e}")),
    }
}

/// One text value: flag, else prompt (interactive), else existing, else `default`.
fn text_value(
    p: &mut dyn Prompter,
    flag: Option<String>,
    question: &str,
    existing: Option<String>,
    default: Option<&str>,
    missing: Option<&'static str>,
) -> Result<String, InitError> {
    if let Some(f) = flag {
        return Ok(f);
    }
    let fallback = existing.or(default.map(String::from));
    if p.interactive() {
        return ask(p, question, fallback.as_deref(), |_| Ok(()));
    }
    match (fallback, missing) {
        (Some(v), _) => Ok(v),
        (None, Some(flag)) => Err(InitError::Required { flag }),
        (None, None) => unreachable!("a value with no default names its flag"),
    }
}

fn set(table: &mut dyn TableLike, key: &str, new: Value) {
    match table.get_mut(key) {
        Some(item) => {
            let mut v = new;
            if let Some(old) = item.as_value() {
                *v.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(v);
        }
        None => {
            table.insert(key, Item::Value(new));
        }
    }
}

fn string_array(items: &[String]) -> Value {
    let mut a = Array::new();
    for i in items {
        a.push(i.as_str());
    }
    Value::Array(a)
}

/// Run the guided setup against the file at `path`. Returns the path written.
pub fn run(
    path: &Path,
    home: &Path,
    vdir_flag: Option<&Path>,
    args: &InitArgs,
    p: &mut dyn Prompter,
) -> Result<PathBuf, InitError> {
    let mut doc: DocumentMut = match fs::read_to_string(path) {
        Ok(t) => t.parse()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };
    let ex = existing_values(&doc);

    let vdir_answer = text_value(
        p,
        vdir_flag.map(|v| v.display().to_string()),
        Q_VDIR,
        ex.vdir.clone(),
        Some(DEFAULT_VDIR),
        None,
    )?;
    let vdir_path = expand_tilde(Path::new(&vdir_answer), home);
    let vdir_text = if vdir_answer.starts_with('~') {
        vdir_answer
    } else {
        home_relative(&vdir_path, home)
    };

    let sync_flags =
        args.url.is_some() || args.username.is_some() || args.password_command.is_some();
    let sync = if args.no_sync {
        false
    } else if sync_flags {
        true
    } else if p.interactive() {
        p.confirm(Q_SYNC, ex.has_sync)?
            .ok_or(InitError::Cancelled)?
    } else {
        ex.has_sync
    };

    let mut conn = None;
    if sync {
        let url = text_value(
            p,
            args.url.clone(),
            Q_URL,
            ex.url.clone(),
            None,
            Some("--url"),
        )?;
        let user = text_value(
            p,
            args.username.clone(),
            Q_USER,
            ex.username.clone(),
            None,
            Some("--username"),
        )?;
        let cmd = match &args.password_command {
            Some(c) => split_command(c).map_err(InitError::BadCommand)?,
            None if p.interactive() => {
                let a = ask(p, Q_PASS, ex.password_command.as_deref(), |s| {
                    split_command(s).map(|_| ())
                })?;
                split_command(&a).expect("validated")
            }
            None => match &ex.password_command {
                Some(c) => split_command(c).map_err(InitError::BadCommand)?,
                None => {
                    return Err(InitError::Required {
                        flag: "--password-command",
                    });
                }
            },
        };
        conn = Some((url, user, cmd));
    }

    // Edit in place; everything else stays as it was.
    set(doc.as_table_mut(), "vdir", Value::from(vdir_text));
    match conn {
        Some((url, user, cmd)) => {
            if !doc.get("sync").is_some_and(Item::is_table_like) {
                doc["sync"] = Item::Table(Table::new());
            }
            let t = doc["sync"].as_table_like_mut().expect("table");
            set(t, "url", Value::from(url));
            set(t, "username", Value::from(user));
            set(t, "password_command", string_array(&cmd));
            if let Some(ca) = &args.ca_file {
                set(t, "ca_file", Value::from(ca.display().to_string()));
            }
            for k in REMOVED {
                t.remove(k);
            }
        }
        None => {
            doc.remove("sync");
        }
    }
    for k in REMOVED {
        doc.remove(k);
    }

    write_through(path, &doc.to_string())?;
    fs::create_dir_all(&vdir_path)?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Answers in order; `None` is EOF. Records every question with its default.
    struct Script {
        interactive: bool,
        answers: VecDeque<Option<&'static str>>,
        asked: Vec<(String, Option<String>)>,
    }

    impl Script {
        fn new(answers: &[Option<&'static str>]) -> Self {
            Script {
                interactive: true,
                answers: answers.iter().copied().collect(),
                asked: vec![],
            }
        }
        fn say(answers: &[&'static str]) -> Self {
            Self::new(&answers.iter().map(|a| Some(*a)).collect::<Vec<_>>())
        }
    }

    impl Prompter for Script {
        fn interactive(&self) -> bool {
            self.interactive
        }
        fn line(&mut self, q: &str, d: Option<&str>) -> io::Result<Option<String>> {
            self.asked.push((q.into(), d.map(String::from)));
            Ok(self
                .answers
                .pop_front()
                .expect("unexpected prompt")
                .map(String::from))
        }
        fn confirm(&mut self, q: &str, d: bool) -> io::Result<Option<bool>> {
            self.asked.push((q.into(), Some(d.to_string())));
            Ok(self
                .answers
                .pop_front()
                .expect("unexpected prompt")
                .map(|a| match a {
                    "" => d,
                    a => a == "y",
                }))
        }
    }

    struct Env {
        dir: tempfile::TempDir,
    }

    impl Env {
        fn new() -> Self {
            Env {
                dir: tempfile::tempdir().unwrap(),
            }
        }
        fn home(&self) -> PathBuf {
            self.dir.path().join("home")
        }
        fn cfg(&self) -> PathBuf {
            self.dir.path().join("config/twydo/config.toml")
        }
        fn init(&self, p: &mut Script) -> Result<PathBuf, InitError> {
            run(&self.cfg(), &self.home(), None, &InitArgs::default(), p)
        }
        fn text(&self) -> String {
            fs::read_to_string(self.cfg()).unwrap()
        }
        fn existing(&self, text: &str) {
            fs::create_dir_all(self.cfg().parent().unwrap()).unwrap();
            fs::write(self.cfg(), text).unwrap();
        }
    }

    #[test]
    fn fresh_file_with_defaults() {
        let e = Env::new();
        e.init(&mut Script::say(&["", "n"])).unwrap();
        assert_eq!(e.text(), "vdir = \"~/.local/share/twydo/vdir\"\n");
        assert!(e.home().join(".local/share/twydo/vdir").is_dir());
    }

    #[test]
    fn fresh_file_with_sync() {
        let e = Env::new();
        let mut p = Script::say(&[
            "",
            "y",
            "https://dav.example.test/a/",
            "alice",
            "pass show \"twydo/caldav x\"",
        ]);
        e.init(&mut p).unwrap();
        let doc: toml_edit::DocumentMut = e.text().parse().unwrap();
        let s = doc["sync"].as_table().unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s["url"].as_str(), Some("https://dav.example.test/a/"));
        assert_eq!(s["username"].as_str(), Some("alice"));
        let cmd: Vec<_> = s["password_command"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(cmd, ["pass", "show", "twydo/caldav x"]);
    }

    const WITH_SYNC: &str = "vdir = \"~/v\"\n\n[sync]\nurl = \"https://dav.example.test/a/\"\nusername = \"alice\"\npassword_command = [\"pass\", \"show\", \"x y\"]\n";

    #[test]
    fn existing_values_are_defaults() {
        let e = Env::new();
        e.existing(WITH_SYNC);
        let mut p = Script::say(&["", "", "", "", ""]);
        e.init(&mut p).unwrap();
        assert_eq!(e.text(), WITH_SYNC);
        assert_eq!(p.asked[1], (Q_SYNC.into(), Some("true".into())));
        assert_eq!(p.asked[4].1.as_deref(), Some("pass show 'x y'"));
    }

    #[test]
    fn old_production_shape_is_cleaned() {
        let e = Env::new();
        e.existing(
            "# my config\nvdir = \"~/v\"\nafter_edit = true\n\n[writes]\nmode = \"on\"\n\n[lists.x]\nname = \"X\"\n\n[sync]\npair = \"p\"\nlocal_storage = \"l\"\nvdirsyncer_config = \"/c\"\nafter_edit = true\ncommand = \"vdirsyncer\"\nlist_python = \"python3\"\n",
        );
        let mut p = Script::say(&["", "y", "https://dav.example.test/a/", "alice", "true"]);
        e.init(&mut p).unwrap();
        assert_eq!(p.asked[2], (Q_URL.into(), None));
        let t = e.text();
        assert!(t.starts_with("# my config\n"));
        for keep in [
            "[writes]",
            "[lists.x]",
            "command = \"vdirsyncer\"",
            "list_python = \"python3\"",
        ] {
            assert!(t.contains(keep), "{keep}\n{t}");
        }
        for gone in ["pair", "local_storage", "vdirsyncer_config", "after_edit"] {
            assert!(!t.contains(gone), "{gone}\n{t}");
        }
    }

    #[test]
    fn declining_sync_removes_the_table() {
        let e = Env::new();
        e.existing(WITH_SYNC);
        e.init(&mut Script::say(&["", "n"])).unwrap();
        assert_eq!(e.text(), "vdir = \"~/v\"\n");
    }

    #[test]
    fn relative_symlink_is_preserved() {
        let e = Env::new();
        let cfgdir = e.cfg().parent().unwrap().to_path_buf();
        fs::create_dir_all(&cfgdir).unwrap();
        let dots = e.dir.path().join("config/dotfiles");
        fs::create_dir_all(&dots).unwrap();
        fs::write(dots.join("config.toml"), "vdir = \"~/old\"\n").unwrap();
        // config/twydo/config.toml -> ../dotfiles/config.toml
        std::os::unix::fs::symlink("../dotfiles/config.toml", e.cfg()).unwrap();
        e.init(&mut Script::say(&["", "n"])).unwrap();
        assert_eq!(
            fs::read_link(e.cfg()).unwrap(),
            Path::new("../dotfiles/config.toml")
        );
        assert_eq!(
            fs::read_to_string(dots.join("config.toml")).unwrap(),
            "vdir = \"~/old\"\n"
        );
    }

    #[test]
    fn tilde_vdir_written_relative() {
        let e = Env::new();
        e.init(&mut Script::say(&["~/tasks", "n"])).unwrap();
        assert_eq!(e.text(), "vdir = \"~/tasks\"\n");
        let abs = e.home().join("more").display().to_string();
        let abs: &'static str = Box::leak(abs.into_boxed_str());
        e.init(&mut Script::say(&[abs, "n"])).unwrap();
        assert_eq!(e.text(), "vdir = \"~/more\"\n");
    }

    #[test]
    fn eof_writes_nothing() {
        let e = Env::new();
        let mut p = Script::new(&[
            Some(""),
            Some("y"),
            Some("https://dav.example.test/a/"),
            None,
        ]);
        assert!(matches!(e.init(&mut p), Err(InitError::Cancelled)));
        assert!(!e.cfg().exists());
    }

    #[test]
    fn unbalanced_quote_reprompts() {
        let e = Env::new();
        let mut p = Script::say(&[
            "",
            "y",
            "https://dav.example.test/a/",
            "alice",
            "pass 'oops",
            "pass show x",
        ]);
        e.init(&mut p).unwrap();
        assert!(
            e.text()
                .contains("password_command = [\"pass\", \"show\", \"x\"]")
        );
    }

    #[test]
    fn non_interactive_uses_existing_then_requires() {
        let e = Env::new();
        let mut p = Script::say(&[]);
        p.interactive = false;
        let args = InitArgs {
            username: Some("alice".into()),
            password_command: Some("true".into()),
            ..InitArgs::default()
        };
        let r = run(&e.cfg(), &e.home(), None, &args, &mut p);
        assert!(matches!(r, Err(InitError::Required { flag: "--url" })));
        assert!(!e.cfg().exists());
    }
}
