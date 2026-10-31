use axiom::session::{Attachment, Config};
use serde_json::{json,Value};
#[test]
fn caller_command_lock_blocks_reentry_across_attachments_and_releases_without_polling() {
    let temp=tempfile::tempdir().unwrap();
    let project=temp.path().join("project");std::fs::create_dir(&project).unwrap();
    let config=Config {home:temp.path().join("home"),bin:std::env::current_exe().unwrap(),codex:Ok(vec!["must-not-launch.exe".into()])};
    let (first,opened)=Attachment::open(&config,&json!({"project":project,"text":"test caller gate"})).unwrap();
    let (second,_)=Attachment::open(&config,&json!({"session":opened["session"]})).unwrap();
    let gate=config.home.join("authority").join(opened["session"].as_str().unwrap()).join("caller-command");
    std::fs::create_dir_all(&gate).unwrap();
    let (started_tx,started_rx)=std::sync::mpsc::channel();
    let (done_tx,done_rx)=std::sync::mpsc::channel();
    let handles=axiom::orch::with_lock(&gate,false,|| {
        let handles:Vec<_>=[first,second].into_iter().enumerate().map(|(i,host)| {
            let started=started_tx.clone();let done=done_tx.clone();
            std::thread::spawn(move || {
                started.send(()).unwrap();
                let value=if i==0 {host.status()} else {host.call("workflow",&json!({"action":"status"}))};
                done.send(value).unwrap();
            })
        }).collect();
        for _ in 0..2 {started_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();}
        assert!(matches!(done_rx.recv_timeout(std::time::Duration::from_millis(100)),Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
        Ok(handles)
    }).unwrap();
    for _ in 0..2 {assert_eq!(done_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap().unwrap()["status"],"completed");}
    for handle in handles {handle.join().unwrap();}
}
#[test]
fn real_host_enforces_research_review_planning_and_restart() {
    let temp=tempfile::tempdir().unwrap();
    let project=temp.path().join("demo");std::fs::create_dir(&project).unwrap();
    let home=temp.path().join("home");let record=temp.path().join("rpc.json");
    let config=Config { home:home.clone(), bin:std::env::current_exe().unwrap(), codex:Ok(vec![
        "python".into(),"-I".into(),format!("{}/tests/session/authority_fake.py",env!("CARGO_MANIFEST_DIR")),
        "--scenario".into(),"role-flow".into(),"--home".into(),home.to_string_lossy().into(),
        "--project".into(),project.to_string_lossy().into(),"--slug".into(),"demo".into(),
        "--record".into(),record.to_string_lossy().into()]) };
    let (host,opened)=Attachment::open(&config,&json!({"project":project,"text":"tiny CLI","skill":"wish"})).unwrap();
    host.call("goal",&json!({"mode":"change"})).unwrap();
    for tool in ["Write","Bash","orch","codex_run"] {
        assert!(host.call(tool,&json!({"actor":"dispatcher"})).is_err(),"{tool}");
    }
    assert!(host.call("Agent",&json!({"subagent_type":"chronos:astra","prompt":"bypass"})).is_err());
    let args=json!({"subagent_type":"chronos:dispatcher","project":"demo","cwd":project,"prompt":"tiny CLI"});
    let first=host.call("Agent",&args).unwrap();
    assert_eq!(first["workflow"]["phase"],"intent_review","{first}");
    let revision=first["workflow"]["revision"].clone();
    assert!(host.call("workflow",&json!({"action":"verdict","revision":0,"decision":"pass"})).is_err());
    assert!(host.call("Agent",&args).is_err());
    host.call("workflow",&json!({"action":"verdict","revision":revision,"decision":"pass"})).unwrap();
    let rpc:Value=serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    assert!(rpc["requests"].as_array().unwrap().iter().any(|r| r["method"]=="thread/unsubscribe"));
    assert!(!rpc["requests"].as_array().unwrap().iter().any(|r| r["method"]=="thread/archive"));
    let session=opened["session"].clone();drop(host);
    let (resumed,_)=Attachment::open(&config,&json!({"session":session})).unwrap();
    assert_eq!(resumed.call("workflow",&json!({"action":"status"})).unwrap()["workflow"]["phase"],"planning");
    // The fake stops after planning and then makes no progress. Rust must keep
    // those continuations inside the same command, then fail boundedly instead
    // of returning a false core boundary or polling the model indefinitely.
    assert_eq!(resumed.call("Agent",&args).unwrap_err().kind,"dispatcher_stalled");
    assert_eq!(resumed.call("workflow",&json!({"action":"status"})).unwrap()["workflow"]["phase"],"execution");
    assert!(resumed.call("workflow",&json!({"action":"submit"})).is_err());
    let rpc:Value=serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    assert_eq!(rpc["requests"].as_array().unwrap().iter().filter(|r|
        r["method"]=="turn/start" && r["params"]["input"][0]["text"].as_str().unwrap_or("").starts_with("Rust has retained this command")
    ).count(),2);
    let starts:Vec<_>=rpc["requests"].as_array().unwrap().iter().filter(|r| r["method"]=="thread/start").collect();
    assert!(starts.iter().all(|r| r["params"]["ephemeral"]==true));
    assert!(starts.iter().any(|r| r["params"]["developerInstructions"].as_str().unwrap_or("").contains("You decide where")));
}
