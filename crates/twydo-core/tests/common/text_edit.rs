//! Synthetic variants of the existing C29 phone text-edit capture.
use super::Env;
use twydo_core::{
    config::WriteMode,
    ical::{Property, parse, serialize},
};

pub const PARENT: &str = "twydo-p8-c29-b";

pub fn env() -> Env {
    Env::new(
        WriteMode::Verified,
        "complete/checklist-reset/phone-items-and-text/list1",
    )
}

pub fn mutate(env: &Env, uid: &str, f: impl FnOnce(&mut twydo_core::ical::Component)) {
    let path = env.file(uid);
    let mut cal = parse(&std::fs::read(&path).unwrap()).unwrap();
    f(cal.root.children_named_mut("VTODO").next().unwrap());
    std::fs::write(path, serialize(&cal)).unwrap();
}

pub fn closed() -> Env {
    let env = env();
    mutate(&env, PARENT, |todo| {
        todo.prop_mut("STATUS").unwrap().set_value("COMPLETED");
        todo.insert_prop(Property::new("COMPLETED", vec![], "20261210T120000Z"), None);
    });
    env
}

/// Compare raw serialized bytes after removing only the four authorized top-level fields.
pub fn assert_preserved(before: &[u8], after: &[u8]) {
    let held = |bytes: &[u8]| {
        let mut cal = parse(bytes).unwrap();
        let t = cal.root.children_named_mut("VTODO").next().unwrap();
        for name in ["SUMMARY", "DESCRIPTION", "DTSTAMP", "LAST-MODIFIED"] {
            t.remove_props(name);
        }
        serialize(&cal)
    };
    assert_eq!(held(before), held(after));
}
