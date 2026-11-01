use std::path::{Path, PathBuf};

use twydo_core::ical::parse;
use twydo_core::model::meta_edit::{MetaJson, default_task_meta};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/phase0/v1")
}

fn meta_value(path: &Path) -> Option<String> {
    let cal = parse(&std::fs::read(path).unwrap()).unwrap();
    cal.root
        .children_named("VTODO")
        .find_map(|t| t.prop("X-2DOAPP-METADATA").map(|p| p.value.clone()))
}

fn value_of(rel: &str) -> String {
    meta_value(&fixtures().join(rel)).unwrap_or_else(|| panic!("no metadata in {rel}"))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "ics") {
            out.push(p);
        }
    }
}

const A: &str = "plan4-list1/01-create/0a04f22aeb294b719633c978fe1975e5.ics";
const B: &str = "plan4-list1/01-create/41b9cdfea48e46fd9da52f9337c3f6c4.ics";
const C: &str = "plan4-list1/01-create/3b93fba16ec74e24b2da63a73c29ca8c.ics";
const D: &str = "plan4-list1/01-create/855a275473c34da9840a6a76c5f18385.ics";

#[test]
fn encode_parse_is_identity_on_fixtures() {
    let mut files = vec![];
    walk(&fixtures(), &mut files);
    let mut checked = 0;
    for f in files {
        let cal = parse(&std::fs::read(&f).unwrap()).unwrap();
        for t in cal.root.children_named("VTODO") {
            for p in t.props_named("X-2DOAPP-METADATA") {
                if !p.value.contains("<2Do Meta>") {
                    continue;
                }
                let m = MetaJson::parse(&p.value).unwrap_or_else(|| panic!("{f:?}"));
                assert_eq!(m.encode(), p.value, "{f:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 20, "only {checked} values checked");
}

#[test]
fn set_keeps_key_order() {
    let base = "rule-months-day-1-28";
    let one = std::fs::read_dir(fixtures().join(base).join("01-create"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "ics"))
        .unwrap();
    let name = one.file_name().unwrap().to_str().unwrap().to_string();
    let v1 = meta_value(&one).unwrap();
    let v3 = value_of(&format!("{base}/03-edit/{name}"));
    let mut m = MetaJson::parse(&v1).unwrap();
    m.set_i64("RecurrenceValue", 2);
    assert_eq!(m.get_i64("RecurrenceValue"), Some(2));
    assert_eq!(m.encode(), v3);
}

#[test]
fn remove_and_place() {
    let a = value_of(A);
    let mut m = MetaJson::parse(&a).unwrap();
    m.remove("StartDate");
    assert_eq!(m.encode(), a);

    m.set_i64("StartDate", 1796083200);
    assert_eq!(m.encode(), value_of(D));
    m.remove("StartDate");
    assert_eq!(m.encode(), a);

    let mut b = MetaJson::parse(&value_of(B)).unwrap();
    b.set_i64("duetime", 1730);
    assert_eq!(b.encode(), value_of(C));
}

#[test]
fn set_str_and_get() {
    let mut m = MetaJson::parse(&value_of(A)).unwrap();
    m.set_str("RUID", "x y");
    assert!(m.encode().contains("%22RUID%22%3A%22x%20y%22"));
    assert_eq!(m.get_i64("RUID"), None);
    assert_eq!(m.get_i64("nope"), None);
}

#[test]
fn default_task_meta_matches_2do() {
    assert_eq!(default_task_meta().encode(), value_of(A));
}

#[test]
fn non_json_is_none() {
    assert!(MetaJson::parse("@tasktype:project").is_none());
    assert!(MetaJson::parse("<2Do Meta>%5B1%5D</2Do Meta>").is_none());
}
