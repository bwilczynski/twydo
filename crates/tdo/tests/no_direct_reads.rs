//! No `tdo` client reads the vdir, the state dir or `config.toml` itself: only the daemon does.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    "vdir::load",
    "existing_vdir_state",
    "ConflictStore",
    "Config::load",
    "resolve_config",
    "store::journal",
    "fs::read",
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Whether `pattern` may appear in `rel` (a path under `src/`).
fn allowed(rel: &str, pattern: &str) -> bool {
    match rel {
        "cli/daemon.rs" | "cli/mod.rs" | "editor.rs" => true,
        "cli/conflicts.rs" => pattern == "resolve_config",
        _ => false,
    }
}

#[test]
fn clients_do_not_read_the_vdir_or_config() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    assert!(!files.is_empty());
    let mut hits = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&src).unwrap().to_string_lossy().into_owned();
        for (n, line) in std::fs::read_to_string(&f).unwrap().lines().enumerate() {
            // Test blocks sit at the end of a file: stop scanning there.
            if line.trim() == "#[cfg(test)]" {
                break;
            }
            for p in FORBIDDEN {
                if line.contains(p) && !allowed(&rel, p) {
                    hits.push(format!("src/{rel}:{}: {p}", n + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "clients read files directly:\n{}",
        hits.join("\n")
    );
}
