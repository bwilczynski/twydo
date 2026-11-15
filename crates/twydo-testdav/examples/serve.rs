//! Serves a fake CalDAV home on localhost until killed: `serve <root> <list-id>...`.
//!
//! Prints the `[sync].url` as its first line. The login is `alice` / `secret`. The README
//! demo (`docs/demo/render.sh`) uses it so that list discovery has a server to ask.

use std::io::Write;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("usage: serve <root> <list-id>..."));
    let server = twydo_testdav::FakeDav::start(&root);
    for id in args {
        twydo_testdav::add_collection(&root, &id, &id, "#3289D2");
    }
    println!("{}", server.url());
    std::io::stdout().flush().expect("flush");
    loop {
        std::thread::park();
    }
}
