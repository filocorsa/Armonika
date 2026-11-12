#[test]
fn version_flag_prints_name_and_version() {
    let out = assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        format!("armonika {}", armonika_core::VERSION)
    );
}
