//! バイナリを実際に起動する統合テスト。board は tempdir に作る。

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_lsboard");

fn lsb(board: &Path) -> Command {
    let mut c = Command::new(BIN);
    c.env_remove("LSBOARD_SID").env("LSBOARD_DIR", board);
    c
}

fn run_ok(cmd: &mut Command) -> String {
    let out = cmd.output().expect("spawn lsboard");
    assert!(
        out.status.success(),
        "command failed: {:?}\nstdout: {}\nstderr: {}",
        cmd,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn new_board() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let b = d.path().join("board");
    run_ok(lsb(&b).arg("init"));
    (d, b)
}

fn names(board: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(board)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

fn ended_names(board: &Path) -> Vec<String> {
    names(&board.join(".ended"))
}

/// run を通さずに entry を作る（hook のテスト用）。
fn seed(board: &Path, sid: &str, status: &str, name: &str) {
    fs::create_dir(board.join(".sids").join(sid)).unwrap();
    fs::write(board.join(".sids").join(sid).join("agent"), "claude").unwrap();
    fs::write(
        board.join(".sids").join(sid).join("pid"),
        std::process::id().to_string(),
    )
    .unwrap();
    fs::File::create(board.join(format!("{status}__{sid}__claude__{name}"))).unwrap();
}

fn hook(board: &Path, sid: &str, input: &str) -> (i32, String, String) {
    let mut child = lsb(board)
        .args(["hook", "claude"])
        .env("LSBOARD_SID", sid)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ev(event: &str, extra: &str) -> String {
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(", {extra}")
    };
    format!(
        r#"{{"session_id": "sess-1", "transcript_path": "/tmp/t.jsonl", "hook_event_name": "{event}"{extra}}}"#
    )
}

#[test]
fn init_set_ls_end_lint_lifecycle() {
    let (_d, b) = new_board();
    assert!(b.join(".lsboard").is_file());
    seed(&b, "k3f9", "40-idle", "worker");

    run_ok(lsb(&b).args(["set", "--sid", "k3f9", "waiting", "need: a/b_c"]));
    assert_eq!(names(&b), ["10-waiting__k3f9__claude__worker need a b c"]);
    assert_eq!(
        fs::read_to_string(b.join(".sids/k3f9/waiting")).unwrap(),
        "agent"
    );

    let ls = run_ok(lsb(&b).arg("ls"));
    assert!(
        ls.contains("10-waiting  k3f9  claude  worker  need a b c"),
        "{ls}"
    );
    let json = run_ok(lsb(&b).args(["ls", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v[0]["sid"], "k3f9");
    assert_eq!(v[0]["message"], "need a b c");

    run_ok(lsb(&b).args(["set", "working"]).env("LSBOARD_SID", "k3f9"));
    assert_eq!(names(&b), ["20-working__k3f9__claude__worker"]);
    assert!(!b.join(".sids/k3f9/waiting").exists());

    assert_eq!(run_ok(lsb(&b).arg("lint")), "");

    run_ok(lsb(&b).args(["end", "k3f9"]));
    assert!(names(&b).is_empty());
    assert_eq!(ended_names(&b), ["90-ended__k3f9__claude__worker"]);

    // ended entry cannot be set again
    let out = lsb(&b)
        .args(["set", "--sid", "k3f9", "idle"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let out = lsb(&b).args(["end", "k3f9"]).output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn ls_tree_shows_family_and_child_count() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "conductor");
    seed(&b, "k3f9.01", "40-idle", "worker");
    seed(&b, "k3f9.01.01", "20-working", "explorer");
    seed(&b, "a1b2", "40-idle", "other");
    let out = run_ok(lsb(&b).args(["ls", "--tree"]));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with("40-idle     a1b2"), "{out}");
    assert!(
        lines[1].contains("k3f9 ") && lines[1].ends_with("[1 live child]"),
        "{out}"
    );
    assert!(lines[2].contains("  k3f9.01 "), "{out}");
    assert!(lines[3].contains("    k3f9.01.01"), "{out}");
}

#[test]
fn lint_reports_problems() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "w");
    fs::File::create(b.join("20-working__k3f9__claude__w again")).unwrap();
    fs::File::create(b.join("garbage.txt")).unwrap();
    fs::File::create(b.join("40-idle__zzzz__claude__x")).unwrap();
    seed(&b, "a1b2.01", "40-idle", "orphan");
    fs::write(b.join(".sids/k3f9/pid"), "999999999").unwrap();
    let out = lsb(&b).arg("lint").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("unparsable entry name: \"garbage.txt\""), "{s}");
    assert!(s.contains("duplicate entries for sid k3f9: 2"), "{s}");
    assert!(s.contains("entry without reservation: zzzz"), "{s}");
    assert!(
        s.contains("child a1b2.01 is live but its parent a1b2 is not"),
        "{s}"
    );
    assert!(s.contains("process gone: k3f9 (pid 999999999)"), "{s}");
}

#[test]
fn hook_is_a_noop_without_sid_and_never_fails() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "40-idle", "w");
    let out = lsb(&b)
        .args(["hook", "claude"])
        .stdin(Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            c.stdin
                .take()
                .unwrap()
                .write_all(b"{\"hook_event_name\":\"UserPromptSubmit\"}")
                .unwrap();
            c.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success());
    assert_eq!(names(&b), ["40-idle__k3f9__claude__w"]);

    // garbage input and unknown sid: still exit 0
    let (code, _, err) = hook(&b, "k3f9", "not json");
    assert_eq!(code, 0);
    assert!(err.contains("parse hook input"));
    let (code, _, _) = hook(&b, "zzzz", &ev("UserPromptSubmit", r#""prompt": "hi""#));
    assert_eq!(code, 0);
    assert_eq!(names(&b), ["40-idle__k3f9__claude__w"]);
}

#[test]
fn hook_claude_transitions() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "40-idle", "w");

    let (code, out, _) = hook(&b, "k3f9", &ev("SessionStart", r#""source": "startup""#));
    assert_eq!(code, 0);
    assert!(out.contains("you are sid k3f9"), "{out}");
    assert!(out.contains("lsboard set waiting"), "{out}");
    assert_eq!(
        fs::read_to_string(b.join(".sids/k3f9/session")).unwrap(),
        "sess-1"
    );
    assert_eq!(
        fs::read_to_string(b.join(".sids/k3f9/transcript")).unwrap(),
        "/tmp/t.jsonl"
    );

    hook(&b, "k3f9", &ev("UserPromptSubmit", r#""prompt": "do it""#));
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);

    let tool = r#""tool_name": "Bash", "tool_input": {"command": "touch x", "description": "d"}"#;
    hook(&b, "k3f9", &ev("PreToolUse", tool));
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);
    hook(&b, "k3f9", &ev("PermissionRequest", tool));
    assert_eq!(names(&b), ["10-waiting__k3f9__claude__w approve Bash"]);
    assert!(
        fs::read_to_string(b.join(".sids/k3f9/waiting"))
            .unwrap()
            .starts_with("hook ")
    );

    // PostToolUse of a different tool does not clear the wait
    hook(
        &b,
        "k3f9",
        &ev(
            "PostToolUse",
            r#""tool_name": "Read", "tool_input": {"file_path": "/x"}"#,
        ),
    );
    assert_eq!(names(&b), ["10-waiting__k3f9__claude__w approve Bash"]);
    // same tool + input (key order differs) clears it
    let same = r#""tool_name": "Bash", "tool_input": {"description": "d", "command": "touch x"}, "tool_response": {}"#;
    hook(&b, "k3f9", &ev("PostToolUse", same));
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);
    assert!(!b.join(".sids/k3f9/waiting").exists());

    // AskUserQuestion: answers added after the fact are ignored in the hash
    let q =
        r#""tool_name": "AskUserQuestion", "tool_input": {"questions": [{"question": "which?"}]}"#;
    hook(&b, "k3f9", &ev("PermissionRequest", q));
    assert_eq!(names(&b), ["10-waiting__k3f9__claude__w question"]);
    let qa = r#""tool_name": "AskUserQuestion", "tool_input": {"questions": [{"question": "which?"}], "answers": {"which?": "red"}, "annotations": {}}"#;
    hook(&b, "k3f9", &ev("PostToolUse", qa));
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);

    // Stop with running background task → background; PreToolUse brings it back
    hook(
        &b,
        "k3f9",
        &ev(
            "Stop",
            r#""background_tasks": [{"id": "x", "status": "running"}]"#,
        ),
    );
    assert_eq!(names(&b), ["30-background__k3f9__claude__w"]);
    hook(&b, "k3f9", &ev("PreToolUse", tool));
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);
    hook(&b, "k3f9", &ev("Stop", r#""background_tasks": []"#));
    assert_eq!(names(&b), ["40-idle__k3f9__claude__w"]);
    // background with nothing running any more → idle
    hook(&b, "k3f9", &ev("UserPromptSubmit", r#""prompt": "go""#));
    hook(
        &b,
        "k3f9",
        &ev(
            "Stop",
            r#""background_tasks": [{"id": "x", "status": "running"}]"#,
        ),
    );
    assert_eq!(names(&b), ["30-background__k3f9__claude__w"]);
    hook(
        &b,
        "k3f9",
        &ev(
            "Stop",
            r#""background_tasks": [{"id": "x", "status": "completed"}]"#,
        ),
    );
    assert_eq!(names(&b), ["40-idle__k3f9__claude__w"]);

    // task-notification wakes an idle entry
    hook(
        &b,
        "k3f9",
        &ev(
            "UserPromptSubmit",
            r#""prompt": "<task-notification>done</task-notification>""#,
        ),
    );
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);

    // hook-set wait is cleared by Stop (denied tool, no PostToolUse)
    hook(&b, "k3f9", &ev("PermissionRequest", tool));
    assert_eq!(names(&b), ["10-waiting__k3f9__claude__w approve Bash"]);
    hook(&b, "k3f9", &ev("Stop", ""));
    assert_eq!(names(&b), ["40-idle__k3f9__claude__w"]);

    // agent-declared wait survives PermissionRequest, Stop and task-notification; a human prompt clears it
    run_ok(lsb(&b).args(["set", "--sid", "k3f9", "waiting", "merge PR 100"]));
    hook(&b, "k3f9", &ev("PermissionRequest", tool));
    hook(&b, "k3f9", &ev("Stop", ""));
    hook(
        &b,
        "k3f9",
        &ev(
            "UserPromptSubmit",
            r#""prompt": "<task-notification>x</task-notification>""#,
        ),
    );
    assert_eq!(names(&b), ["10-waiting__k3f9__claude__w merge PR 100"]);
    hook(&b, "k3f9", &ev("UserPromptSubmit", r#""prompt": "ok go""#));
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);

    // SessionEnd: only prompt_input_exit / logout end self; clear and other just touch
    for reason in ["clear", "other", "bypass_permissions_disabled"] {
        hook(
            &b,
            "k3f9",
            &ev("SessionEnd", &format!(r#""reason": "{reason}""#)),
        );
        assert_eq!(
            names(&b),
            ["20-working__k3f9__claude__w"],
            "reason {reason}"
        );
    }
    hook(
        &b,
        "k3f9",
        &ev("SessionEnd", r#""reason": "prompt_input_exit""#),
    );
    assert!(names(&b).is_empty());
    assert_eq!(ended_names(&b), ["90-ended__k3f9__claude__w"]);
}

#[test]
fn hook_claude_subagents() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "conductor");
    let sub = r#""agent_id": "agent-abc", "agent_type": "General-Purpose""#;
    hook(&b, "k3f9", &ev("SubagentStart", sub));
    assert_eq!(
        names(&b),
        [
            "20-working__k3f9.01__claude__general-purpose",
            "20-working__k3f9__claude__conductor"
        ]
    );
    assert_eq!(
        fs::read_to_string(b.join(".sids/k3f9.01/agent_id")).unwrap(),
        "agent-abc"
    );
    // duplicate start is ignored
    hook(&b, "k3f9", &ev("SubagentStart", sub));
    assert_eq!(names(&b).len(), 2);

    // tool events carrying agent_id target the child
    let tool = r#""agent_id": "agent-abc", "tool_name": "Bash", "tool_input": {"command": "ls"}"#;
    hook(&b, "k3f9", &ev("PermissionRequest", tool));
    assert!(
        names(&b)
            .contains(&"10-waiting__k3f9.01__claude__general-purpose approve Bash".to_string())
    );
    assert!(names(&b).contains(&"20-working__k3f9__claude__conductor".to_string()));
    hook(&b, "k3f9", &ev("PostToolUse", tool));
    assert!(names(&b).contains(&"20-working__k3f9.01__claude__general-purpose".to_string()));

    hook(
        &b,
        "k3f9",
        &ev("SubagentStop", r#""agent_id": "agent-abc""#),
    );
    assert_eq!(names(&b), ["20-working__k3f9__claude__conductor"]);
    assert_eq!(
        ended_names(&b),
        ["90-ended__k3f9.01__claude__general-purpose"]
    );

    // a second subagent gets .02. A child started with `run --parent` has no agent_id.
    hook(
        &b,
        "k3f9",
        &ev("SubagentStart", r#""agent_id": "agent-def""#),
    );
    assert!(names(&b).contains(&"20-working__k3f9.02__claude__subagent".to_string()));
    seed(&b, "k3f9.03", "20-working", "worker-process");
    // SessionEnd(clear) ends the subagent, keeps self and the process child
    hook(&b, "k3f9", &ev("SessionEnd", r#""reason": "clear""#));
    assert_eq!(
        names(&b),
        [
            "20-working__k3f9.03__claude__worker-process",
            "20-working__k3f9__claude__conductor"
        ]
    );
    assert!(ended_names(&b).contains(&"90-ended__k3f9.02__claude__subagent".to_string()));
    // logout ends self too, still not the process child
    hook(&b, "k3f9", &ev("SessionEnd", r#""reason": "logout""#));
    assert_eq!(names(&b), ["20-working__k3f9.03__claude__worker-process"]);
}

#[test]
fn hook_subagent_start_clips_long_names_and_cleans_up_on_failure() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "conductor");
    let long = "x".repeat(200);
    hook(
        &b,
        "k3f9",
        &ev(
            "SubagentStart",
            &format!(r#""agent_id": "long-1", "agent_type": "{long}""#),
        ),
    );
    let n = names(&b);
    let child = n
        .iter()
        .find(|x| x.contains("__k3f9.01__"))
        .expect("child created");
    assert_eq!(
        *child,
        format!("20-working__k3f9.01__claude__{}", "x".repeat(64))
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // make the board dir unwritable so create() fails; .sids stays writable
        fs::set_permissions(&b, fs::Permissions::from_mode(0o555)).unwrap();
        let (code, _, err) = hook(
            &b,
            "k3f9",
            &ev(
                "SubagentStart",
                r#""agent_id": "fail-1", "agent_type": "explorer""#,
            ),
        );
        fs::set_permissions(&b, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(code, 0);
        assert!(err.contains("subagent entry for k3f9.02"), "{err}");
        assert!(
            !b.join(".sids/k3f9.02").exists(),
            "reservation must be released"
        );
        // later events for that agent_id fall back to the parent instead of vanishing
        hook(
            &b,
            "k3f9",
            &ev(
                "PermissionRequest",
                r#""agent_id": "fail-1", "tool_name": "Bash", "tool_input": {}"#,
            ),
        );
        assert!(
            names(&b).contains(&"10-waiting__k3f9__claude__conductor approve Bash".to_string())
        );
    }
}

#[test]
fn run_creates_entry_passes_env_and_ends_it() {
    let (_d, b) = new_board();
    let bin_dir = Path::new(BIN).parent().unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let marker = _d.path().join("seen");
    let script = format!(
        "lsboard set waiting 'need you'; lsboard ls > '{}'; echo sid=$LSBOARD_SID; exit 3",
        marker.display()
    );
    let out = lsb(&b)
        .env("PATH", path)
        .args(["run", "--name", "Test Worker", "--", "sh", "-c", &script])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let sid = stdout
        .trim()
        .strip_prefix("sid=")
        .expect("sid line")
        .to_string();
    assert_eq!(sid.len(), 4);
    let seen = fs::read_to_string(&marker).unwrap();
    assert!(
        seen.contains(&format!("10-waiting  {sid}  sh  test-worker  need you")),
        "{seen}"
    );
    assert!(names(&b).is_empty());
    assert_eq!(
        ended_names(&b),
        [format!("90-ended__{sid}__sh__test-worker")]
    );
    assert_eq!(
        fs::read_to_string(b.join(".sids").join(&sid).join("agent")).unwrap(),
        "sh"
    );
    assert!(b.join(".sids").join(&sid).join("pid").is_file());

    // --parent reserves a child
    let out = lsb(&b)
        .args(["run", "--parent", &sid, "--", "true"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(ended_names(&b).contains(&format!("90-ended__{sid}.01__true__true")));

    let out = lsb(&b)
        .args(["run", "--", "/nonexistent/agent"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(127));
}

#[test]
fn run_claude_registers_hooks_via_settings() {
    let (_d, b) = new_board();
    // a fake `claude` that prints its argv and the settings file
    let fake = _d.path().join("claude");
    fs::write(&fake, "#!/bin/sh\necho \"$@\"\ncat \"$2\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = lsb(&b)
        .args(["run", "--"])
        .arg(&fake)
        .arg("--resume")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.starts_with("--settings "), "{s}");
    assert!(s.contains("--resume"), "{s}");
    let json_start = s.find('{').unwrap();
    let v: serde_json::Value = serde_json::from_str(&s[json_start..]).unwrap();
    assert!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with("hook claude")
    );
    assert_eq!(v["hooks"]["PostToolUse"][0]["matcher"], "*");
    assert_eq!(ended_names(&b).len(), 1);
    assert!(ended_names(&b)[0].ends_with("__claude__claude"));
}

#[test]
fn wait_returns_when_a_child_needs_attention() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "conductor");
    seed(&b, "k3f9.01", "20-working", "worker");
    seed(&b, "k3f9.02", "20-working", "worker");

    let b2 = b.clone();
    let started = Instant::now();
    let waiter = thread::spawn(move || {
        lsb(&b2)
            .args(["wait", "--children-of", "k3f9", "--timeout", "15"])
            .output()
            .unwrap()
    });
    thread::sleep(Duration::from_millis(600));
    run_ok(lsb(&b).args(["set", "--sid", "k3f9.01", "working"])); // no-op transition
    run_ok(lsb(&b).args(["set", "--sid", "k3f9.02", "waiting", "approve merge"]));
    let out = waiter.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    let s = String::from_utf8_lossy(&out.stdout);
    assert_eq!(s.trim(), "k3f9.02 10-waiting worker approve merge");

    // already satisfied: returns at once with the current state
    let out = lsb(&b)
        .args(["wait", "--timeout", "5"])
        .env("LSBOARD_SID", "k3f9")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "k3f9.02 10-waiting worker approve merge"
    );

    // --all: not everyone is done → waits → times out
    let out = lsb(&b)
        .args(["wait", "--children-of", "k3f9", "--all", "--timeout", "0.5"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));

    // --all satisfied after the other child ends: only live children count
    run_ok(lsb(&b).args(["end", "k3f9.01"]));
    let out = lsb(&b)
        .args([
            "wait",
            "--children-of",
            "k3f9",
            "--all",
            "--timeout",
            "5",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["sid"], "k3f9.02");
    assert_eq!(lines[0]["status"], "10-waiting");

    // no scope at all is an error
    let out = lsb(&b).args(["wait", "--timeout", "1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}

/// 行を別スレッドで読み、期限内に届かなければ None。
fn read_line_within(rx: &std::sync::mpsc::Receiver<String>, d: Duration) -> Option<String> {
    rx.recv_timeout(d).ok()
}

#[test]
fn watch_streams_changes() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "conductor");
    seed(&b, "k3f9.01", "20-working", "worker");
    let mut child = lsb(&b)
        .args(["watch", "--children-of", "k3f9"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    thread::sleep(Duration::from_millis(500));
    let apply = || {
        run_ok(lsb(&b).args(["set", "--sid", "k3f9.01", "idle"]));
        if !b.join(".sids/k3f9.02").exists() {
            seed(&b, "k3f9.02", "20-working", "reviewer");
        }
    };
    apply();
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut reapplied = false;
    while got.len() < 2 && Instant::now() < deadline {
        match read_line_within(&rx, Duration::from_secs(3)) {
            Some(l) => got.push(l),
            None if !reapplied => {
                // the watcher may have taken its first snapshot after our change; nudge again
                run_ok(lsb(&b).args(["set", "--sid", "k3f9.01", "working"]));
                thread::sleep(Duration::from_millis(1200));
                apply();
                reapplied = true;
            }
            None => break,
        }
    }
    child.kill().unwrap();
    let _ = child.wait();
    assert_eq!(
        got,
        ["k3f9.01 40-idle worker", "k3f9.02 20-working reviewer"]
    );
}

#[test]
fn wait_ignores_children_that_ended_before_it_started() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "conductor");
    seed(&b, "k3f9.01", "20-working", "worker");
    seed(&b, "k3f9.02", "20-working", "worker");
    run_ok(lsb(&b).args(["end", "k3f9.01"]));
    assert_eq!(ended_names(&b), ["90-ended__k3f9.01__claude__worker"]);

    // must not return at once with the stale 90-ended line
    let out = lsb(&b)
        .args(["wait", "--children-of", "k3f9", "--timeout", "1.5"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "");

    // a live child ending while waiting is still reported (live → ended transition)
    let b2 = b.clone();
    let waiter = thread::spawn(move || {
        lsb(&b2)
            .args(["wait", "--children-of", "k3f9", "--timeout", "15"])
            .output()
            .unwrap()
    });
    thread::sleep(Duration::from_millis(600));
    run_ok(lsb(&b).args(["end", "k3f9.02"]));
    let out = waiter.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "k3f9.02 90-ended worker"
    );

    // --all with no live children at all → exit 2 and a message, not an instant success
    let out = lsb(&b)
        .args(["wait", "--children-of", "k3f9", "--all", "--timeout", "5"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no live children of k3f9"));

    // --timeout inf / nan / negative must not panic
    seed(&b, "k3f9.03", "40-idle", "worker");
    for t in ["--timeout=inf", "--timeout=nan", "--timeout=-1"] {
        let out = lsb(&b)
            .args(["wait", "--children-of", "k3f9", t])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "timeout {t}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn wait_coalesces_to_one_line_per_sid_and_flushes_on_deadline() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "conductor");
    seed(&b, "k3f9.01", "20-working", "worker");
    let b2 = b.clone();
    let waiter = thread::spawn(move || {
        lsb(&b2)
            .args(["wait", "--children-of", "k3f9", "--timeout", "15"])
            .output()
            .unwrap()
    });
    thread::sleep(Duration::from_millis(600));
    // two transitions inside the 1 s window: only the final state must be printed
    run_ok(lsb(&b).args(["set", "--sid", "k3f9.01", "waiting", "first"]));
    thread::sleep(Duration::from_millis(150));
    run_ok(lsb(&b).args(["set", "--sid", "k3f9.01", "idle"]));
    let out = waiter.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "k3f9.01 40-idle worker"
    );

    // deadline shorter than the coalescing window: buffered events are flushed, then exit 2
    run_ok(lsb(&b).args(["set", "--sid", "k3f9.01", "working"]));
    let b3 = b.clone();
    let waiter = thread::spawn(move || {
        lsb(&b3)
            .args([
                "wait",
                "--children-of",
                "k3f9",
                "--until",
                "waiting",
                "--timeout",
                "0.7",
            ])
            .output()
            .unwrap()
    });
    thread::sleep(Duration::from_millis(300));
    seed(&b, "k3f9.02", "20-working", "reviewer"); // appearance, not in --until
    let out = waiter.join().unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "k3f9.02 20-working reviewer"
    );
}

#[test]
fn end_fails_loudly_on_a_held_lock() {
    let (_d, b) = new_board();
    seed(&b, "k3f9", "20-working", "w");
    fs::create_dir(b.join(".sids/k3f9/lock")).unwrap();
    let out = lsb(&b).args(["end", "k3f9"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("lock is held"));
    assert_eq!(names(&b), ["20-working__k3f9__claude__w"]);
    assert!(
        b.join(".sids/k3f9/lock").is_dir(),
        "stale lock must not be broken automatically"
    );
}

#[test]
fn run_reports_an_entry_it_could_not_end() {
    let (_d, b) = new_board();
    let bin_dir = Path::new(BIN).parent().unwrap();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = lsb(&b)
        .env("PATH", path)
        .args([
            "run",
            "--",
            "sh",
            "-c",
            "mkdir \"$LSBOARD_DIR/.sids/$LSBOARD_SID/lock\"; exit 5",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("entry was left on the board"), "{err}");
    assert!(err.contains("40-idle__"), "{err}");
    assert_eq!(names(&b).len(), 1);
}

#[test]
fn run_agent_override_and_no_adapter_notice() {
    let (_d, b) = new_board();
    let wrapper = _d.path().join("my-wrapper");
    fs::write(&wrapper, "#!/bin/sh\necho \"$1\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = lsb(&b)
        .args(["run", "--agent", "claude", "--"])
        .arg(&wrapper)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "--settings");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("no hook adapter"));
    assert!(ended_names(&b)[0].ends_with("__claude__my-wrapper"));

    let out = lsb(&b).args(["run", "--", "true"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "lsboard run: no hook adapter for true; only start/end will be recorded"
    );
    let out = lsb(&b)
        .args(["run", "--no-hooks", "--", "true"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "");
}
