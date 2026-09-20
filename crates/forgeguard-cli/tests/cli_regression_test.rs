use std::{
    fs,
    process::{self, Command},
    time::{SystemTime, UNIX_EPOCH},
};

fn temporary_directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "forgeguard-cli-{label}-{}-{}",
        process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow the Unix epoch")
            .as_nanos()
    ))
}

#[test]
fn unavailable_update_is_not_reported_as_success() {
    let home = temporary_directory("update-home-file");
    fs::write(&home, "not a directory").expect("temporary home file should be created");

    for arguments in [
        vec!["update", "--json"],
        vec!["update", "--check", "--json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_forgeguard"))
            .args(arguments)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("PATH", "")
            .output()
            .expect("update command should run");

        assert!(!output.status.success());
        let response: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("update output should be JSON");
        assert_eq!(response["status"], "unknown");
    }
    fs::remove_file(home).expect("temporary home file should be removed");
}

#[test]
fn json_init_reports_mcp_registration_failure_on_stderr() {
    let root = temporary_directory("invalid-mcp");
    fs::create_dir_all(&root).expect("temporary project should be created");
    fs::write(root.join(".mcp.json"), "not json").expect("invalid MCP config should be created");

    let output = Command::new(env!("CARGO_BIN_EXE_forgeguard"))
        .args([
            "--root",
            root.to_str().expect("temporary path should be UTF-8"),
            "init",
            "--agent",
            "claude",
            "--mcp",
            "--no-index",
            "--json",
        ])
        .output()
        .expect("init command should run");

    assert!(output.status.success());
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("init stdout should remain valid JSON");
    assert!(String::from_utf8_lossy(&output.stderr).contains("MCP registration skipped"));
    fs::remove_dir_all(root).expect("temporary project should be removed");
}
