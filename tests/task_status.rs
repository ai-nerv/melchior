use melchior::scratch::Scratch;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct Agent {
    process: Child,
    _out: BufReader<ChildStdout>,
    id: String,
    parent: Option<String>,
}

fn command(runtime: &Path, id: &str, parent: Option<&str>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_melchior"));
    command
        .current_dir(runtime)
        .env("HOME", runtime)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("XDG_CONFIG_HOME", runtime.join("config"))
        .env("XDG_DATA_HOME", runtime.join("data"))
        .env("MAGI_MELCHIOR_PROJECT", "poll")
        .env("MAGI_MELCHIOR_ROLE", "main")
        .env("MAGI_MELCHIOR_ID", id)
        .env_remove("MAGI_MELCHIOR_PARENT")
        .env_remove("MAGI_MELCHIOR_TOKEN");
    if let Some(parent) = parent {
        command.env("MAGI_MELCHIOR_PARENT", parent);
    }
    command
}

impl Agent {
    fn start(runtime: &Path, id: &str, parent: Option<&str>) -> Self {
        let mut process = command(runtime, id, parent)
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("serve starts");
        let mut line = String::new();
        let mut out = BufReader::new(process.stdout.take().expect("stdout"));
        out.read_line(&mut line).expect("listening event");
        let event: serde_json::Value = serde_json::from_str(&line).expect("JSON event");
        assert_eq!(event["event"], "listening", "{line}");
        Self {
            process,
            _out: out,
            id: id.into(),
            parent: parent.map(str::to_owned),
        }
    }

    fn tool(&self, runtime: &Path, args: &[&str]) -> String {
        let output = command(runtime, &self.id, self.parent.as_deref())
            .arg("tool")
            .args(args)
            .output()
            .expect("tool runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("UTF-8 output")
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[test]
fn a_childs_report_does_not_make_its_running_turn_complete() {
    let runtime = Scratch::new("mt", "poll");
    let parent = Agent::start(&runtime, "parent", None);
    let mut child = Agent::start(&runtime, "psi-zeta", Some("parent"));
    let input = child.process.stdin.as_mut().expect("stdin");
    writeln!(
        input,
        r#"{{"event":"doing","busy":true,"phase":"working","working_for":4}}"#
    )
    .expect("working event");
    input.flush().expect("flush");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = parent.tool(&runtime, &["--verb=status", "--who=psi-zeta"]);
        let status: serde_json::Value = serde_json::from_str(status.trim()).expect("status JSON");
        if status["busy"] == true {
            break;
        }
        assert!(Instant::now() < deadline, "child did not become busy");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.tool(
        &runtime,
        &[
            "--verb=send",
            "--who=parent",
            "--message=preliminary findings",
        ],
    );
    let inbox = parent.tool(&runtime, &["--verb=inbox"]);
    assert!(inbox.contains("preliminary findings"), "{inbox}");
    let status = parent.tool(
        &runtime,
        &["--verb=task", "--who=psi-zeta", "--about=psi-zeta"],
    );
    assert!(status.contains("\"busy\":true"), "{status}");
    assert!(status.contains("\"phase\":\"working\""), "{status}");
    assert!(status.contains("not an `ask` task"), "{status}");
    assert!(!status.contains("completed"), "{status}");
}

#[test]
fn reporting_notifies_the_parent_with_a_revision_and_preserves_the_whole_body() {
    let runtime = Scratch::new("mt", "report");
    let parent = Agent::start(&runtime, "parent", None);
    let child = Agent::start(&runtime, "psi-zeta", Some("parent"));
    let body = "No findings. The scan failed before returning any observations.\n".repeat(1200);
    child.tool(&runtime, &["--verb=report", &format!("--message={body}")]);
    let inbox = parent.tool(&runtime, &["--verb=inbox"]);
    assert!(inbox.contains("report"), "{inbox}");
    let report = parent.tool(
        &runtime,
        &["--verb=report", "--who=psi-zeta", "--about=json"],
    );
    let value: serde_json::Value = serde_json::from_str(&report).expect("report JSON");
    assert_eq!(value["report"], body);
    let revision = value["revision"].as_str().expect("revision string");
    assert_eq!(revision.len(), 64);
    assert!(inbox.contains(revision), "{inbox}");
    assert!(!melchior::wire::Sort::Report.interrupts());
}

#[test]
fn identical_large_reports_on_separate_turns_have_distinct_receipts() {
    let runtime = Scratch::new("mt", "repeat");
    let parent = Agent::start(&runtime, "parent", None);
    let child = Agent::start(&runtime, "psi-zeta", Some("parent"));
    let body = "No findings from this scan.\n".repeat(8000);
    let mut revisions = Vec::new();
    for _ in 0..2 {
        let mut submit = command(&runtime, &child.id, child.parent.as_deref())
            .args(["tool", "--verb=report", "--about=-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("stdin report submission");
        submit
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(body.as_bytes())
            .expect("large report input");
        let output = submit.wait_with_output().expect("submission completes");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let read = parent.tool(
            &runtime,
            &["--verb=report", "--who=psi-zeta", "--about=json"],
        );
        let report: serde_json::Value = serde_json::from_str(&read).expect("report JSON");
        assert_eq!(report["report"], body);
        revisions.push(report["revision"].as_str().expect("revision").to_owned());
    }
    assert_ne!(
        revisions[0], revisions[1],
        "identical reports from different turns are not duplicates"
    );
    let inbox = parent.tool(&runtime, &["--verb=inbox"]);
    for revision in revisions {
        assert!(
            inbox.contains(&revision),
            "each submission must notify the parent"
        );
    }
}
