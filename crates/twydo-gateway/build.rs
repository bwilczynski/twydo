//! Embeds the Vite build output (`web/dist`) into the gateway binary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

const PLACEHOLDER_HTML: &str = "<!doctype html><meta charset=\"utf-8\"><title>Twydo</title>\
<p>The Twydo web app was not built; run <code>npm run build</code> in web/.</p>";

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("read web/dist")
        .map(|e| e.expect("dir entry").path())
        .collect();
    entries.sort();
    for p in entries {
        println!("cargo:rerun-if-changed={}", p.display());
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, p));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=TWYDO_REQUIRE_WEB");
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let mut code = String::new();
    let placeholder;
    if dist.is_dir() {
        placeholder = false;
        let dist = dist.canonicalize().unwrap();
        let mut files = Vec::new();
        walk(&dist, &dist, &mut files);
        code.push_str("pub const ASSETS: &[Asset] = &[\n");
        for (rel, abs) in files {
            writeln!(
                code,
                "    Asset {{ path: {rel:?}, mime: {:?}, bytes: include_bytes!({:?}) }},",
                mime(&rel),
                abs.to_string_lossy()
            )
            .unwrap();
        }
        code.push_str("];\n");
    } else {
        if env::var("TWYDO_REQUIRE_WEB").as_deref() == Ok("1") {
            panic!("web/dist is missing and TWYDO_REQUIRE_WEB=1; run `npm run build` in web/");
        }
        placeholder = true;
        let p = out_dir.join("placeholder.html");
        fs::write(&p, PLACEHOLDER_HTML).unwrap();
        writeln!(
            code,
            "pub const ASSETS: &[Asset] = &[Asset {{ path: \"index.html\", mime: \"text/html; charset=utf-8\", bytes: include_bytes!({:?}) }}];",
            p.to_string_lossy()
        )
        .unwrap();
    }
    writeln!(code, "pub const PLACEHOLDER: bool = {placeholder};").unwrap();
    fs::write(out_dir.join("assets.rs"), code).unwrap();
}
