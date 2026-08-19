use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_docrev"))
}

fn temp_document() -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic.xlsx");
    // unique per test, not per process: under `cargo test` all tests share
    // one process and a pid-based name makes them race on the same file
    let dest = std::env::temp_dir().join(format!("docrev-cli-{}.xlsx", uuid::Uuid::new_v4()));
    std::fs::copy(&source, &dest).unwrap();
    let _ = std::fs::remove_file(sidecar_of(&dest));
    dest
}

fn sidecar_of(document: &Path) -> PathBuf {
    let mut p = document.as_os_str().to_owned();
    p.push(".docrev.json");
    PathBuf::from(p)
}

fn cleanup(document: &Path) {
    let _ = std::fs::remove_file(sidecar_of(document));
    let mut lock = sidecar_of(document).into_os_string();
    lock.push(".lock");
    let _ = std::fs::remove_file(lock);
    let _ = std::fs::remove_file(document);
}

#[test]
fn full_agent_loop() {
    let doc = temp_document();

    let out = bin()
        .args(["comment", "add"])
        .arg(&doc)
        .args([
            "--cell",
            "売上!B3",
            "--body",
            "check this",
            "--author",
            "claude",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let thread: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(thread["anchor"]["cell"], "B3");
    assert_eq!(thread["author"], "claude");
    let id = thread["id"].as_str().unwrap().to_string();

    let out = bin()
        .args(["comment", "reply"])
        .arg(&doc)
        .args(["--thread", &id, "--body", "done"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let replied: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(replied["replies"][0]["author"], "agent", "default author");

    let out = bin()
        .args(["comment", "list"])
        .arg(&doc)
        .args(["--json", "--unresolved"])
        .output()
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed["version"], 1, "list output uses the sidecar schema");
    assert_eq!(listed["comments"].as_array().unwrap().len(), 1);
    // the thread carries its cell content, so agents act without dumping
    let first = &listed["comments"][0];
    assert_eq!(first["cell"]["value"], "80");
    assert_eq!(first["cell"]["row"]["A3"], "みかん");
    assert_eq!(first["cell"]["row"]["C3"], "5");

    let out = bin()
        .args(["comment", "resolve"])
        .arg(&doc)
        .args(["--thread", &id])
        .output()
        .unwrap();
    let resolved: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(resolved["resolved"], true);

    let out = bin()
        .args(["comment", "list"])
        .arg(&doc)
        .args(["--json", "--unresolved"])
        .output()
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed["comments"].as_array().unwrap().len(), 0);

    cleanup(&doc);
}

#[test]
fn invalid_input_fails_with_nonzero_exit() {
    let doc = temp_document();

    let out = bin()
        .args(["comment", "add"])
        .arg(&doc)
        .args(["--cell", "nope", "--body", "x"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid cell reference"));

    let out = bin()
        .args(["comment", "add"])
        .arg(&doc)
        .args(["--cell", "架空!B3", "--body", "x"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found"));

    let out = bin()
        .args(["comment", "reply"])
        .arg(&doc)
        .args(["--thread", "bogus", "--body", "x"])
        .output()
        .unwrap();
    assert!(!out.status.success());

    let out = bin()
        .args(["comment", "list", "no-such-file.xlsx"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("document not found"));

    cleanup(&doc);
}
