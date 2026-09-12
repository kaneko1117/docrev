use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_docrev"))
}

fn temp_document() -> PathBuf {
    temp_copy_of("basic.xlsx")
}

fn temp_copy_of(fixture: &str) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture);
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
    assert_eq!(listed["version"], 2, "list output uses the sidecar schema");
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
fn a_formatted_number_lists_its_stored_value() {
    let doc = temp_copy_of("formats.xlsx");
    let out = bin()
        .args(["comment", "add"])
        .arg(&doc)
        .args(["--cell", "書式!A1", "--body", "check", "--author", "agent"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = bin()
        .args(["comment", "list"])
        .arg(&doc)
        .arg("--json")
        .output()
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let cell = &listed["comments"][0]["cell"];
    assert_eq!(cell["value"], "15%");
    assert_eq!(cell["raw"], serde_json::json!(0.15));
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

fn temp_markdown(text: &str) -> PathBuf {
    let dest = std::env::temp_dir().join(format!("docrev-cli-{}.md", uuid::Uuid::new_v4()));
    std::fs::write(&dest, text).unwrap();
    let _ = std::fs::remove_file(sidecar_of(&dest));
    dest
}

fn run(args: &[&str], doc: &Path) -> std::process::Output {
    let mut command = bin();
    command.arg(args[0]);
    if args[0] == "comment" {
        command.arg(args[1]).arg(doc).args(&args[2..]);
    } else {
        command.arg(doc).args(&args[1..]);
    }
    command.output().unwrap()
}

#[test]
fn markdown_agent_loop() {
    let doc = temp_markdown("# docrev\n\nOpen a document.\n\n- Excel only\n");

    let out = run(&["dump"], &doc);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "     1\t# docrev\n     2\t\n     3\tOpen a document.\n     4\t\n     5\t- Excel only\n"
    );

    let out = run(
        &[
            "comment",
            "add",
            "--line",
            "5",
            "--body",
            "not any more",
            "--author",
            "user",
        ],
        &doc,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let thread: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        thread["anchor"],
        serde_json::json!({"kind": "line", "line": 5})
    );
    let id = thread["id"].as_str().unwrap().to_string();

    let out = run(&["comment", "add", "--line", "5", "--body", "agreed"], &doc);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let same: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(same["id"], id, "one line, one thread");
    assert_eq!(same["replies"][0]["author"], "agent");

    let out = run(&["comment", "list", "--json"], &doc);
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed["version"], 2);
    let first = &listed["comments"][0];
    assert_eq!(first["line"]["text"], "- Excel only");
    assert_eq!(
        first["line"]["context"],
        serde_json::json!({"3": "Open a document.", "4": ""})
    );
    assert!(first.get("cell").is_none() && first.get("hidden").is_none());
    assert_eq!(listed["workbook_comments"], serde_json::json!([]));

    let out = run(&["comment", "list"], &doc);
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("● line 5 [user] not any more"));

    std::fs::write(&doc, "# docrev\n").unwrap();
    let out = run(&["comment", "list", "--json"], &doc);
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed["comments"][0]["hidden"], true);
    assert!(listed["comments"][0].get("line").is_none());

    cleanup(&doc);
}

#[test]
fn markdown_rejects_targets_that_do_not_fit() {
    let doc = temp_markdown("one\ntwo\n");
    let stderr = |out: std::process::Output| {
        assert!(!out.status.success());
        String::from_utf8_lossy(&out.stderr).to_string()
    };

    let err = stderr(run(&["comment", "add", "--line", "3", "--body", "x"], &doc));
    assert!(
        err.contains("line 3 is beyond the end of the file (2 lines)"),
        "{err}"
    );
    let err = stderr(run(&["comment", "add", "--line", "0", "--body", "x"], &doc));
    assert!(err.contains("1.."), "clap rejects 0: {err}");
    let err = stderr(run(
        &["comment", "add", "--cell", "s!A1", "--body", "x"],
        &doc,
    ));
    assert!(err.contains("use --line"), "{err}");
    let err = stderr(run(&["comment", "add", "--body", "x"], &doc));
    assert!(err.contains("--cell") && err.contains("--line"), "{err}");
    let err = stderr(run(&["comment", "list", "--sheet", "s"], &doc));
    assert!(err.contains("--sheet does not apply"), "{err}");
    let err = stderr(run(&["dump", "--formulas"], &doc));
    assert!(err.contains("--formulas does not apply"), "{err}");
    let err = stderr(run(&["dump", "--sheet", "s"], &doc));
    assert!(err.contains("no sheets"), "{err}");
    assert!(!sidecar_of(&doc).exists(), "nothing was written");

    let xlsx = temp_document();
    let err = stderr(run(
        &["comment", "add", "--line", "1", "--body", "x"],
        &xlsx,
    ));
    assert!(err.contains("use --cell"), "{err}");

    let txt = doc.with_extension("txt");
    std::fs::write(&txt, "plain\n").unwrap();
    let err = stderr(run(&["dump"], &txt));
    assert!(err.contains("unsupported file type \".txt\""), "{err}");
    assert!(err.contains(".md"), "{err}");
    let _ = std::fs::remove_file(&txt);

    cleanup(&doc);
    cleanup(&xlsx);
}
