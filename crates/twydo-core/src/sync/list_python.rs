//! Which Python interpreter runs the bundled collection helper. The helper imports
//! vdirsyncer, so the interpreter has to be one vdirsyncer is installed for. When
//! `[sync].list_python` is unset, that is vdirsyncer's own interpreter, read from the shebang
//! of the `[sync].command` executable.

use std::ffi::OsStr;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::config::SyncConfig;
use crate::which::is_executable;

/// Used when the setting is unset and vdirsyncer's interpreter cannot be read.
pub const DEFAULT: &str = "python3";

/// Where the interpreter came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `[sync].list_python`.
    Configured,
    /// The shebang of the `[sync].command` executable.
    Vdirsyncer,
    /// `python3` on PATH.
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Source::Configured => "[sync].list_python",
            Source::Vdirsyncer => "vdirsyncer's interpreter",
            Source::Default => "default",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPython {
    /// Program to run: an absolute path, or a name looked up on PATH.
    pub program: String,
    pub source: Source,
}

impl fmt::Display for ListPython {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.program, self.source)
    }
}

/// The interpreter for `sync`, looking `[sync].command` up on this process's PATH.
pub fn resolve(sync: &SyncConfig) -> ListPython {
    resolve_with(sync, std::env::var_os("PATH").as_deref())
}

fn resolve_with(sync: &SyncConfig, path: Option<&OsStr>) -> ListPython {
    if let Some(program) = &sync.list_python {
        return ListPython {
            program: program.clone(),
            source: Source::Configured,
        };
    }
    match find_command(&sync.command, path)
        .and_then(|c| c.canonicalize().ok())
        .and_then(|c| shebang_python(&c))
    {
        Some(program) => ListPython {
            program,
            source: Source::Vdirsyncer,
        },
        None => ListPython {
            program: DEFAULT.into(),
            source: Source::Default,
        },
    }
}

/// `command` itself when it names a path, else its first executable file on `path`.
fn find_command(command: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    if command.is_empty() {
        return None;
    }
    if command.contains('/') {
        return Some(PathBuf::from(command));
    }
    crate::which::on(command, path)
}

/// The interpreter a script names, when it is an existing absolute python:
/// - `#!/abs/path/python[3[.N]] [args]`;
/// - pip's launcher for paths too long for a shebang: `#!/bin/sh`, then
///   `'''exec' "/abs/path/python" "$0" "$@"` (unquoted when the path has no spaces).
///
/// `#!/usr/bin/env …`, other interpreters, a missing shebang or a binary give `None`.
fn shebang_python(file: &Path) -> Option<String> {
    let mut head = Vec::with_capacity(1024);
    std::fs::File::open(file)
        .ok()?
        .take(1024)
        .read_to_end(&mut head)
        .ok()?;
    let mut lines = head.strip_prefix(b"#!")?.split(|&b| b == b'\n');
    let first = std::str::from_utf8(lines.next()?).ok()?;
    let interpreter = first.split_whitespace().next()?;
    if Path::new(interpreter).file_name() == Some(OsStr::new("sh")) {
        let second = std::str::from_utf8(lines.next()?).ok()?;
        let rest = second.strip_prefix("'''exec' ")?;
        let launched = match rest.strip_prefix('"') {
            Some(quoted) => &quoted[..quoted.find('"')?],
            None => rest.split_whitespace().next()?,
        };
        return python_at(launched);
    }
    python_at(interpreter)
}

/// `interpreter` when it is an absolute, executable python, python3 or python3.N.
fn python_at(interpreter: &str) -> Option<String> {
    let p = Path::new(interpreter);
    let name = p.file_name()?.to_str()?;
    let python = name == "python"
        || name
            .strip_prefix("python3")
            .is_some_and(|v| v.is_empty() || v.strip_prefix('.').is_some_and(is_digits));
    (p.is_absolute() && python && is_executable(p)).then(|| interpreter.to_string())
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct Fx {
        dir: tempfile::TempDir,
    }

    impl Fx {
        fn new() -> Self {
            let fx = Fx {
                dir: tempfile::tempdir().unwrap(),
            };
            fs::create_dir_all(fx.p("bin")).unwrap();
            fs::create_dir_all(fx.p("py")).unwrap();
            fx
        }

        fn p(&self, rel: &str) -> PathBuf {
            self.dir.path().join(rel)
        }

        fn exe(&self, rel: &str, body: &[u8]) -> PathBuf {
            let p = self.p(rel);
            fs::write(&p, body).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            p
        }

        /// An executable `py/<name>` standing in for an interpreter.
        fn python(&self, name: &str) -> String {
            let p = self.exe(&format!("py/{name}"), b"");
            p.to_str().unwrap().to_string()
        }

        /// `bin/vdirsyncer` with `first_line`, then resolve with PATH = `bin`.
        fn resolve(&self, first_line: &str) -> ListPython {
            self.exe(
                "bin/vdirsyncer",
                format!("{first_line}\nimport x\n").as_bytes(),
            );
            self.resolve_existing()
        }

        /// `bin/vdirsyncer` with exactly `script`, then resolve with PATH = `bin`.
        fn resolve_script(&self, script: &str) -> ListPython {
            self.exe("bin/vdirsyncer", script.as_bytes());
            self.resolve_existing()
        }

        fn resolve_existing(&self) -> ListPython {
            let sync = SyncConfig::default();
            resolve_with(&sync, Some(self.p("bin").as_os_str()))
        }
    }

    fn default() -> ListPython {
        ListPython {
            program: DEFAULT.into(),
            source: Source::Default,
        }
    }

    fn from_vdirsyncer(program: &str) -> ListPython {
        ListPython {
            program: program.into(),
            source: Source::Vdirsyncer,
        }
    }

    #[test]
    fn a_configured_interpreter_wins() {
        let fx = Fx::new();
        let py = fx.python("python");
        fx.exe("bin/vdirsyncer", format!("#!{py}\n").as_bytes());
        let sync = SyncConfig {
            list_python: Some("/opt/my/python".into()),
            ..SyncConfig::default()
        };
        assert_eq!(
            resolve_with(&sync, Some(fx.p("bin").as_os_str())),
            ListPython {
                program: "/opt/my/python".into(),
                source: Source::Configured,
            }
        );
    }

    #[test]
    fn an_absolute_python_shebang_is_used() {
        let fx = Fx::new();
        let py = fx.python("python");
        assert_eq!(fx.resolve(&format!("#!{py}")), from_vdirsyncer(&py));
    }

    #[test]
    fn shebang_arguments_are_ignored() {
        let fx = Fx::new();
        let py = fx.python("python3.13");
        assert_eq!(fx.resolve(&format!("#!{py} -E -s")), from_vdirsyncer(&py));
    }

    #[test]
    fn env_python3_falls_back() {
        let fx = Fx::new();
        assert_eq!(fx.resolve("#!/usr/bin/env python3"), default());
    }

    #[test]
    fn env_split_falls_back() {
        let fx = Fx::new();
        assert_eq!(fx.resolve("#!/usr/bin/env -S python3 -E"), default());
    }

    #[test]
    fn a_non_python_interpreter_falls_back() {
        let fx = Fx::new();
        let sh = fx.python("sh");
        assert_eq!(fx.resolve(&format!("#!{sh}")), default());
    }

    #[test]
    fn a_missing_interpreter_falls_back() {
        let fx = Fx::new();
        let gone = fx.p("py/python3");
        assert_eq!(fx.resolve(&format!("#!{}", gone.display())), default());
    }

    #[test]
    fn a_binary_falls_back() {
        let fx = Fx::new();
        fx.exe("bin/vdirsyncer", b"\x7fELF\x02\x01\x01\0\0\0");
        assert_eq!(fx.resolve_existing(), default());
    }

    #[test]
    fn an_empty_file_falls_back() {
        let fx = Fx::new();
        fx.exe("bin/vdirsyncer", b"");
        assert_eq!(fx.resolve_existing(), default());
    }

    #[test]
    fn no_vdirsyncer_on_path_falls_back() {
        let fx = Fx::new();
        assert_eq!(fx.resolve_existing(), default());
    }

    #[test]
    fn pips_long_path_launcher_is_read() {
        let fx = Fx::new();
        let long = format!("py/{}/{}", "v".repeat(150), "w".repeat(150));
        fs::create_dir_all(fx.p(&long)).unwrap();
        let py = fx.exe(&format!("{long}/python"), b"");
        let py = py.to_str().unwrap();
        let script = format!("#!/bin/sh\n'''exec' \"{py}\" \"$0\" \"$@\"\n' '''\nimport x\n");
        assert!(script.len() > 256);
        assert_eq!(fx.resolve_script(&script), from_vdirsyncer(py));
    }

    #[test]
    fn pips_unquoted_launcher_is_read() {
        let fx = Fx::new();
        let py = fx.python("python3.12");
        let script = format!("#!/bin/sh\n'''exec' {py} \"$0\" \"$@\"\n' '''\n");
        assert_eq!(fx.resolve_script(&script), from_vdirsyncer(&py));
    }

    #[test]
    fn another_sh_script_falls_back() {
        let fx = Fx::new();
        let py = fx.python("python3");
        let script = format!("#!/bin/sh\nexec {py} -m vdirsyncer \"$@\"\n");
        assert_eq!(fx.resolve_script(&script), default());
    }

    #[test]
    fn the_display_names_the_source() {
        let p = ListPython {
            program: "/usr/bin/python".into(),
            source: Source::Vdirsyncer,
        };
        assert_eq!(p.to_string(), "/usr/bin/python (vdirsyncer's interpreter)");
    }

    #[test]
    fn a_symlink_chain_is_followed() {
        // Homebrew: bin/vdirsyncer -> Cellar/…/libexec/bin/vdirsyncer, a script whose
        // shebang names the libexec python.
        let fx = Fx::new();
        fs::create_dir_all(fx.p("cellar/libexec/bin")).unwrap();
        let py = fx.exe("cellar/libexec/bin/python", b"");
        let py = py.to_str().unwrap();
        fx.exe(
            "cellar/libexec/bin/vdirsyncer",
            format!("#!{py}\nimport x\n").as_bytes(),
        );
        symlink(fx.p("cellar/libexec/bin/vdirsyncer"), fx.p("cellar/link")).unwrap();
        symlink(fx.p("cellar/link"), fx.p("bin/vdirsyncer")).unwrap();
        assert_eq!(fx.resolve_existing(), from_vdirsyncer(py));
    }

    #[test]
    fn a_command_with_a_slash_is_not_looked_up() {
        let fx = Fx::new();
        let py = fx.python("python3");
        let vd = fx.exe("elsewhere", format!("#!{py}\n").as_bytes());
        let sync = SyncConfig {
            command: vd.to_str().unwrap().into(),
            ..SyncConfig::default()
        };
        assert_eq!(resolve_with(&sync, None), from_vdirsyncer(&py));
    }
}
