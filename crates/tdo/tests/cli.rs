use std::process::Command;

#[test]
fn version_flag_prints_name() {
    let out = Command::new(env!("CARGO_BIN_EXE_tdo"))
        .arg("--version")
        .output()
        .expect("run tdo");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("tdo "), "stdout was {stdout:?}");
}
