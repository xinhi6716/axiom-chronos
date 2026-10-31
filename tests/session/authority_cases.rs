use axiom::session::{Attachment, Config};
use serde_json::{json,Value};
#[path = "../common/mod.rs"]
mod common;
#[test]
fn killed_cli_owner_recovers_research_and_replays_without_duplicate_models() {
    crash_recovery_probe(0);
}
#[test]
#[ignore = "explicit 310-second transport-limit acceptance"]
fn long_wait_then_process_loss_recovers_without_duplicate_models() {
    crash_recovery_probe(310);
}
fn crash_recovery_probe(wait_seconds:u64) {
    let temp=tempfile::tempdir().unwrap();
    let project=temp.path().join("demo");std::fs::create_dir(&project).unwrap();
    let home=temp.path().join("home");let record=temp.path().join("rpc.json");
    let marker=temp.path().join("research-ready");
    let codex=vec![common::python(),"-I".into(),format!("{}/tests/session/authority_fake.py",env!("CARGO_MANIFEST_DIR")),
        "--scenario".into(),"role-flow".into(),"--home".into(),home.to_string_lossy().into_owned(),
        "--project".into(),project.to_string_lossy().into_owned(),"--slug".into(),"demo".into(),
        "--record".into(),record.to_string_lossy().into_owned(),"--pause-research".into(),marker.to_string_lossy().into_owned()];
    let config=Config {home:home.clone(),bin:env!("CARGO_BIN_EXE_axiom").into(),codex:Ok(codex.clone())};
    let (host,opened)=Attachment::open(&config,&json!({"project":project,"text":"tiny CLI"})).unwrap();
    host.call("goal",&json!({"mode":"change"})).unwrap();drop(host);
    let request=temp.path().join("request.json");
    std::fs::write(&request,json!({"session":opened["session"],"request_id":"research","tool":"Agent","arguments":{
        "subagent_type":"chronos:dispatcher","project":"demo","cwd":project,"prompt":"tiny CLI"}}).to_string()).unwrap();
    let command=|| {
        let mut c=std::process::Command::new(env!("CARGO_BIN_EXE_axiom"));
        c.arg("session-call").arg(&request).env("AXIOM_HOME",&home).env("AXIOM_DATA_HOME",&home)
            .env("CLAUDE_PLUGIN_DATA",&home).env("AXIOM_CODEX_CMD",serde_json::to_string(&codex).unwrap());c
    };
    let mut child=command().stdout(std::process::Stdio::null()).spawn().unwrap();
    let until=std::time::Instant::now()+std::time::Duration::from_secs(20);
    while !marker.exists() && std::time::Instant::now()<until {
        assert!(child.try_wait().unwrap().is_none(),"CLI exited before crash point");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(),"research never reached crash point");
    let started=std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_secs(wait_seconds));
    assert!(child.try_wait().unwrap().is_none(),"long call ended before release");
    child.kill().unwrap();child.wait().unwrap();
    std::fs::write(marker.with_extension("release"),"go").unwrap();
    let recovered=command().output().unwrap();
    assert!(recovered.status.success(),"{} {}",String::from_utf8_lossy(&recovered.stdout),String::from_utf8_lossy(&recovered.stderr));
    let result:Value=serde_json::from_slice(&recovered.stdout).unwrap();
    assert_eq!(result["workflow"]["phase"],"intent_review","{result}");
    let before=std::fs::read(&record).unwrap();
    let replay=command().output().unwrap();assert!(replay.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&replay.stdout).unwrap(),result);
    assert_eq!(std::fs::read(&record).unwrap(),before,"replay launched a model");
    eprintln!("recovery_probe wait_seconds={wait_seconds} elapsed_ms={} completed_revision={} replay_model_calls=0",started.elapsed().as_millis(),result["workflow"]["revision"]);
}
#[test]
fn durable_request_replay_survives_attachment_loss_and_rejects_changed_input() {
    let temp=tempfile::tempdir().unwrap();
    let config=Config {home:temp.path().join("home"),bin:std::env::current_exe().unwrap(),codex:Ok(vec!["must-not-launch.exe".into()])};
    let (host,opened)=Attachment::open(&config,&json!({"project":temp.path(),"text":"durable transport"})).unwrap();
    let args=json!({"mode":"change"});
    let first=host.call_recorded("mode","goal",&args).unwrap();
    drop(host);
    let (host,_)=Attachment::open(&config,&json!({"session":opened["session"]})).unwrap();
    assert_eq!(host.call_recorded("mode","goal",&args).unwrap(),first);
    axiom::goal::set_status(&config.home,opened["session"].as_str().unwrap(),"blocked",Some("transport acceptance")).unwrap();
    let (replay_host,reopened)=Attachment::open(&config,&json!({"session":opened["session"],"preserve_goal":true})).unwrap();
    assert_eq!(reopened["goal"]["status"],"blocked");
    assert_eq!(replay_host.call_recorded("mode","goal",&args).unwrap(),first);
    assert_eq!(axiom::goal::load(&config.home,opened["session"].as_str().unwrap()).unwrap().unwrap().status,"blocked");
    assert_eq!(host.call_recorded("mode","goal",&json!({"mode":"review"})).unwrap_err().kind,"request_conflict");
    assert_eq!(host.call_recorded("../escape","goal",&args).unwrap_err().kind,"invalid_argument");
    // A begun engineering request with no durable result must never be repeated.
    let dir=config.home.join("authority").join(opened["session"].as_str().unwrap());
    std::fs::write(dir.join("state.json"),json!({"phase":"execution","revision":1,"research":null,"reason":""}).to_string()).unwrap();
    let dispatch=json!({"subagent_type":"chronos:dispatcher","prompt":"continue"});
    std::fs::write(dir.join("request-lost.json"),json!({"input":{"tool":"Agent","arguments":dispatch},"response":null}).to_string()).unwrap();
    assert_eq!(host.call_recorded("lost","Agent",&dispatch).unwrap_err().kind,"recovery_required");
    // Completion saved in authority before a lost transport response is replayed.
    std::fs::write(dir.join("state.json"),json!({"phase":"intent_review","revision":2,"research":null,"reason":""}).to_string()).unwrap();
    let recovered=host.call_recorded("lost","Agent",&dispatch).unwrap();
    assert_eq!(recovered["workflow"]["revision"],2);
    assert_eq!(recovered["replayed"],true);
}
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
        common::python(),"-I".into(),format!("{}/tests/session/authority_fake.py",env!("CARGO_MANIFEST_DIR")),
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
