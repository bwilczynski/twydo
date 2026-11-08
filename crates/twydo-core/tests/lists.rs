use std::fs;
use twydo_core::config::Config;
use twydo_core::lists::{ListService, prefs};

fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let vdir = d.path().join("vdir");
    fs::create_dir_all(vdir.join("dots.id")).unwrap();
    fs::write(vdir.join("dots.id/displayname"), "Synthetic\n").unwrap();
    fs::write(vdir.join("dots.id/task.ics"), b"synthetic held bytes\r\n").unwrap();
    let path = d.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "# keep my comments\nvdir = {:?}\nstate_dir = {:?}\n\n[lists.\"dots.id\"]\n# keep this too\nshow_in_today = false\narchived = false # local preference\n\n[[groups]]\nname = \"Group\"\nlists = [\"dots.id\"]\n",
            vdir.to_str().unwrap(), d.path().join("state").to_str().unwrap()
        ),
    ).unwrap();
    (d, vdir, path)
}

#[test]
fn archive_preserves_config_text_and_task_bytes() {
    let (_d, vdir, path) = setup();
    let before = fs::read(&path).unwrap();
    let cfg = prefs::set_archived(&path, &vdir, "dots.id", true, &before).unwrap();
    assert!(cfg.list_prefs("dots.id").archived);
    assert!(!cfg.list_prefs("dots.id").show_in_today);
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after,
        String::from_utf8(before)
            .unwrap()
            .replace("archived = false", "archived = true")
    );
    assert_eq!(
        fs::read(vdir.join("dots.id/task.ics")).unwrap(),
        b"synthetic held bytes\r\n"
    );
    let bytes = after.into_bytes();
    prefs::set_archived(&path, &vdir, "dots.id", false, &bytes).unwrap();
    assert!(!Config::load(&path).unwrap().list_prefs("dots.id").archived);
}

#[test]
fn archive_stale_config_is_not_overwritten() {
    let (_d, vdir, path) = setup();
    let before = fs::read(&path).unwrap();
    let changed = String::from_utf8(before.clone()).unwrap() + "# external change\n";
    fs::write(&path, &changed).unwrap();
    assert!(prefs::set_archived(&path, &vdir, "dots.id", true, &before).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), changed);
}

#[test]
fn archive_wrong_vdir_and_unsafe_id_are_refused() {
    let (d, vdir, path) = setup();
    let before = fs::read(&path).unwrap();
    let other = d.path().join("other");
    fs::create_dir(&other).unwrap();
    assert!(prefs::set_archived(&path, &other, "dots.id", true, &before).is_err());
    for id in ["../dots.id", "", ".", "missing"] {
        assert!(prefs::set_archived(&path, &vdir, id, true, &before).is_err());
    }
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn archive_adds_quoted_preference_without_losing_groups() {
    let (_d, vdir, path) = setup();
    let old = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        old.replace("archived = false # local preference\n", ""),
    )
    .unwrap();
    let before = fs::read(&path).unwrap();
    let cfg = prefs::set_archived(&path, &vdir, "dots.id", true, &before).unwrap();
    assert!(cfg.list_prefs("dots.id").archived);
    assert_eq!(cfg.groups[0].lists, ["dots.id"]);
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains("# keep this too")
    );
}

#[test]
fn archive_creates_a_preference_table_when_none_exists() {
    let (d, vdir, path) = setup();
    let text = format!(
        "# keep\nvdir = {:?}\nstate_dir = {:?}\n",
        vdir.to_str().unwrap(),
        d.path().join("state").to_str().unwrap()
    );
    fs::write(&path, &text).unwrap();
    let config = prefs::set_archived(&path, &vdir, "dots.id", true, text.as_bytes()).unwrap();
    assert!(config.list_prefs("dots.id").archived);
    assert!(fs::read_to_string(path).unwrap().starts_with(&text));
}

#[cfg(unix)]
#[test]
fn archive_retains_private_config_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let (_d, vdir, path) = setup();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    prefs::set_archived(&path, &vdir, "dots.id", true, &fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn archive_refuses_symlinked_config_or_collection() {
    use std::os::unix::fs::symlink;
    let (d, vdir, path) = setup();
    let before = fs::read(&path).unwrap();
    let alias = d.path().join("alias.toml");
    symlink(&path, &alias).unwrap();
    assert!(prefs::set_archived(&alias, &vdir, "dots.id", true, &before).is_err());
    symlink(vdir.join("dots.id"), vdir.join("alias")).unwrap();
    assert!(prefs::set_archived(&path, &vdir, "alias", true, &before).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn service_holds_config_and_list_preconditions() {
    let (_d, _vdir, path) = setup();
    let cfg = Config::load(&path).unwrap();
    let mut service = ListService::open(&cfg, Some(&path)).unwrap();
    let held = service.state("dots.id").unwrap();
    let changed = service.set_archived("dots.id", true, &held).unwrap();
    assert!(changed.archived);
    assert!(service.set_archived("dots.id", false, &held).is_err());
    assert!(
        !service
            .set_archived("dots.id", false, &changed)
            .unwrap()
            .archived
    );
}

#[test]
fn service_without_config_cannot_create_user_config() {
    let (_d, _vdir, path) = setup();
    let cfg = Config::load(&path).unwrap();
    let mut service = ListService::open(&cfg, None).unwrap();
    let held = service.state("dots.id").unwrap();
    let before = fs::read(&path).unwrap();
    assert!(service.set_archived("dots.id", true, &held).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn a_new_service_rejects_a_card_held_before_an_external_archive_change() {
    let (_d, vdir, path) = setup();
    let old_config = Config::load(&path).unwrap();
    let old_card = ListService::open(&old_config, Some(&path))
        .unwrap()
        .state("dots.id")
        .unwrap();
    prefs::set_archived(&path, &vdir, "dots.id", true, &fs::read(&path).unwrap()).unwrap();
    let mut worker = ListService::open(&old_config, Some(&path)).unwrap();
    assert!(worker.set_archived("dots.id", true, &old_card).is_err());
}
