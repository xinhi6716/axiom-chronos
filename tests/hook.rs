use axiom::{goal, hook};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";
fn event(kind: &str) -> Value {
    json!({"hook_event_name":kind,"session_id":"s"})
}
fn tool(name: &str, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":name,"tool_input":input})
}
fn sub(mut e: Value) -> Value {
    e["agent_id"] = json!("a");
    e
}
fn open(home: &Path) {
    goal::open(home, "s", "wish", None).unwrap();
}
fn call(home: &Path, e: &Value) -> Option<Value> {
    hook::handle(e, Some(home), false)
}
fn denial(v: Option<Value>) -> String {
    let v = v.expect("expected denial");
    let out = &v["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PreToolUse");
    assert_eq!(out["permissionDecision"], "deny");
    out["permissionDecisionReason"].as_str().unwrap().to_owned()
}
fn prompt(text: &str) -> Value {
    let mut e = event("UserPromptSubmit");
    e["prompt"] = json!(text);
    e
}
fn question(n: usize) -> Value {
    tool("AskUserQuestion", json!({"questions":vec![json!({}); n]}))
}
fn dispatch(name: &str) -> Value {
    tool(name, json!({"subagent_type":"chronos:dispatcher"}))
}

#[test]
fn wish_context_and_headless() {
    let d = tempfile::tempdir().unwrap();
    let mut e = prompt("/chronos:wish 修理");
    e["cwd"] = json!(d.path().join("my-project").to_string_lossy());
    e["session_id"] = json!("sess-42");
    for headless in [false, true] {
        let v = hook::handle(&e, Some(d.path()), headless).unwrap();
        let s = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        for part in [
            "sess-42",
            "my-project",
            "Reply in the language of the wish.",
        ] {
            assert!(s.contains(part), "{s}");
        }
        assert!(goal::load(d.path(), "sess-42").unwrap().is_some());
        assert_eq!(
            s.contains("This run is headless: ask the user nothing."),
            headless
        );
    }
}
#[test]
fn stop_active_done_blocked_and_reason() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    assert_eq!(call(h, &event("Stop")).unwrap()["decision"], "block");
    assert!(goal::set_status(h, "s", "blocked", None).is_err());
    goal::set_status(h, "s", "blocked", Some("need access")).unwrap();
    assert!(call(h, &event("Stop")).is_none());
    goal::set_status(h, "s", "done", None).unwrap();
    assert!(call(h, &event("Stop")).is_none());
}
#[test]
fn amendments_reopen_and_preserve_text() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    goal::set_status(h, "s", "done", None).unwrap();
    for text in ["不要停", "可以把 X 改成 Y 嗎"] {
        let v = call(h, &prompt(text)).unwrap();
        let context = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        for part in [
            "amends",
            "precedence over the plan",
            "amend, answer or stop",
        ] {
            assert!(context.contains(part));
        }
        let g = goal::load(h, "s").unwrap().unwrap();
        assert_eq!(g.status, "active");
        assert_eq!(g.amendments.last().unwrap(), text);
        assert_eq!(g.interjection.unwrap().prompt, text);
    }
}
#[test]
fn notification_and_no_goal_prompt() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    assert!(call(h, &prompt("hello")).is_none());
    open(h);
    let before = std::fs::read(h.join("goals/s.json")).unwrap();
    assert!(call(h, &prompt("<task-notification>done</task-notification>")).is_none());
    assert_eq!(before, std::fs::read(h.join("goals/s.json")).unwrap());
    assert!(call(h, &prompt("hello")).is_some());
}
#[test]
fn stop_reason_pending_only() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    let sentence =
        "Classify the pending interjection with the bridge goal tool as amend, answer or stop.";
    assert!(!hook::stop_reason("s", &goal::load(h, "s").unwrap().unwrap()).contains(sentence));
    call(h, &prompt("hello"));
    assert!(hook::stop_reason("s", &goal::load(h, "s").unwrap().unwrap()).contains(sentence));
}
#[test]
fn question_limit() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    denial(call(h, &question(5)));
    assert!(!goal::load(h, "s").unwrap().unwrap().asked);
    assert!(call(h, &question(4)).is_none());
    assert!(goal::load(h, "s").unwrap().unwrap().asked);
    denial(call(h, &question(1)));
}
#[test]
fn question_headless() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    denial(hook::handle(&question(1), Some(h), true));
    assert!(call(h, &question(1)).is_none());
}
#[test]
fn question_subagent() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    denial(call(h, &sub(question(1))));
    assert!(call(h, &question(1)).is_none());
}
#[test]
fn dispatch_agent_and_task() {
    for name in ["Agent", "Task"] {
        let d = tempfile::tempdir().unwrap();
        let h = d.path();
        open(h);
        goal::set_mode(h, "s", "change").unwrap();
        denial(call(h, &sub(dispatch(name))));
        assert!(!goal::load(h, "s").unwrap().unwrap().dispatched);
        assert!(call(h, &dispatch(name)).is_none());
        assert!(goal::load(h, "s").unwrap().unwrap().dispatched);
        denial(call(h, &question(1)));
    }
}
#[test]
fn mode_restrictions() {
    for mode in [None, Some("review"), Some("answer"), Some("change")] {
        let d = tempfile::tempdir().unwrap();
        let h = d.path();
        open(h);
        if let Some(mode) = mode {
            goal::set_mode(h, "s", mode).unwrap();
        }
        for name in ["Agent", "Task"] {
            for agent in [false, true] {
                let e = dispatch(name);
                let e = if agent { sub(e) } else { e };
                if mode == Some("change") && !agent {
                    assert!(call(h, &e).is_none());
                } else {
                    denial(call(h, &e));
                }
            }
        }
        let writable = sub(tool(CODEX, json!({"sandbox":"workspace-write"})));
        if mode == Some("change") {
            assert!(call(h, &writable).is_none());
        } else {
            denial(call(h, &writable));
        }
        assert!(call(h, &sub(tool(CODEX, json!({"sandbox":"read-only"})))).is_none());
        if matches!(mode, Some("review" | "answer")) {
            for agent in [false, true] {
                for e in [
                    tool("Write", json!({"file_path":h.join("../project/file")})),
                    tool("Bash", json!({"command":"sed -i s/a/b/ f"})),
                ] {
                    denial(call(h, &if agent { sub(e) } else { e }));
                }
                let e = tool("Write", json!({"file_path":h.join("inside")}));
                assert!(call(h, &if agent { sub(e) } else { e }).is_none());
            }
        }
    }
}
#[test]
fn nested_agents_only_reviewer() {
    let d = tempfile::tempdir().unwrap();
    let reason = "Inside an agent only chronos:reviewer may be started. Call codex_run yourself; never forward work through another agent.";
    for name in ["Agent", "Task"] {
        for input in [
            json!({}),
            json!({"subagent_type":null}),
            json!({"subagent_type":"chronos:dispatcher"}),
            json!({"subagent_type":"chronos:reviewer "}),
        ] {
            let e = sub(tool(name, input));
            assert_eq!(denial(hook::handle(&e, None, false)), reason);
            assert_eq!(denial(call(d.path(), &e)), reason);
        }
        let reviewer = sub(tool(name, json!({"subagent_type":"chronos:reviewer"})));
        assert!(hook::handle(&reviewer, None, false).is_none());
        assert!(call(d.path(), &reviewer).is_none());
        assert!(call(d.path(), &dispatch(name)).is_none());
    }
}
#[test]
fn hook_never_creates_memory() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    for e in [
        prompt("/chronos:wish test"),
        prompt("more"),
        event("Stop"),
        tool("Write", json!({"file_path":"outside"})),
        tool("Bash", json!({"command":"ls"})),
        dispatch("Agent"),
    ] {
        call(h, &e);
        assert!(!h.join("memory").exists());
    }
}
#[test]
fn core_codex_always_denied() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    denial(call(h, &tool(CODEX, json!({"sandbox":"read-only"}))));
    open(h);
    denial(call(h, &tool(CODEX, json!({"sandbox":"read-only"}))));
    assert!(call(h, &sub(tool(CODEX, json!({"sandbox":"read-only"})))).is_none());
}
#[test]
fn ordinary_sessions_untouched() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    for e in [
        tool("Write", json!({"file_path":"outside"})),
        tool("Bash", json!({"command":"rm x"})),
    ] {
        assert!(call(h, &e).is_none());
        assert!(hook::handle(&e, None, false).is_none());
        open(h);
        denial(call(h, &e));
        std::fs::remove_file(h.join("goals/s.json")).unwrap();
    }
}
#[test]
fn core_write_paths() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    for name in ["Write", "Edit", "NotebookEdit"] {
        let mut e = tool(name, json!({"file_path":"sub/../ok"}));
        e["cwd"] = json!(h.to_string_lossy());
        assert!(call(h, &e).is_none());
        e["tool_input"]["file_path"] = json!("../outside");
        denial(call(h, &e));
    }
}
#[test]
fn bash_contract() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    open(h);
    let bad = [
        "ls >/dev/null; rm x",
        "ls 2>/dev/null | tee f",
        "cat a >/dev/null && rm x",
        "echo `rm x`",
        "echo $(rm x)",
        "find . -exec rm {} +",
        "find . -fprint f",
        "find . -delete",
        "ls && touch x",
        "ls || touch x",
        "ls\ntouch x",
        "FOO=1 rm x",
        "echo x > f",
        "git commit -am x",
        "rm -rf src",
        "cat a; touch b",
        "echo x > f.txt",
        "git diff --output=f",
        "git log --output f",
        "git show --output=f",
        "find . -ok echo {} ;",
        "find . -okdir echo {} ;",
        "find . -fls f",
        "rg --pre cat pattern",
        "rg --pre=cat pattern",
    ];
    let good = [
        "ls 2>/dev/null",
        "FOO=1 ls",
        "echo x 2>&1",
        "cat a >/dev/null",
        "git diff -- src | head -50",
        "find . -print",
        "git status",
        "ls -la",
        "git diff",
        "git log",
        "git show",
        "git rev-parse HEAD",
        "git ls-files",
        "git blame f",
        "ls",
        "cat f",
        "head f",
        "tail f",
        "wc f",
        "pwd",
        "echo x",
        "grep x f",
        "rg x",
        "which git",
        "type git",
    ];
    for (commands, expected) in [(&bad[..], false), (&good[..], true)] {
        for c in commands {
            assert_eq!(hook::read_only_bash(c), expected, "{c}");
            let e = tool("Bash", json!({"command":c}));
            if expected {
                assert!(call(h, &e).is_none(), "{c}");
            } else {
                denial(call(h, &e));
            }
            assert!(hook::handle(&e, None, false).is_none(), "{c}");
            let empty = tempfile::tempdir().unwrap();
            assert!(call(empty.path(), &e).is_none(), "{c}");
        }
    }
}
#[test]
fn stdin_invalid_and_unknown_exit_zero() {
    if std::env::var_os("HOOK_STDIN_CHILD").is_some() {
        std::process::exit(hook::main_stdin());
    }
    let d = tempfile::tempdir().unwrap();
    for input in ["{invalid", "{\"hook_event_name\":\"unknown\"}",
        "{\"hook_event_name\":\"UserPromptSubmit\",\"session_id\":\"sess-42\",\"prompt\":\"/chronos:wish test\"}"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "stdin_invalid_and_unknown_exit_zero",
                "--nocapture",
            ])
            .env("HOOK_STDIN_CHILD", "1")
            .env("AXIOM_HEADLESS", "1")
            .env("AXIOM_HOME", d.path())
            .env("AXIOM_DATA_HOME", d.path())
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
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(0));
        let stdout = output.stdout.strip_prefix(b"\nrunning 1 test\n").unwrap();
        if input.contains("UserPromptSubmit") {
            assert!(output.stderr.is_empty());
            let reply: Value = serde_json::from_slice(stdout).unwrap();
            assert!(reply["hookSpecificOutput"]["additionalContext"].as_str().unwrap()
                .contains("This run is headless: ask the user nothing."));
        } else {
            assert_eq!(String::from_utf8_lossy(&output.stderr).lines().count(), 1);
            assert!(stdout.is_empty());
        }
    }
}
