use std::process::Command;

#[test]
fn version_flag_prints_the_package_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("fairlead {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn doctor_runs_and_names_the_platform() {
    let out = Command::new(env!("CARGO_BIN_EXE_fairlead"))
        .arg("doctor")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout)
        .contains(&format!("platform: {}-", std::env::consts::OS)));
}
