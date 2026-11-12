use super::background::ServiceBackground;
use crate::{
    desktop::{
        ux::seams::{BackgroundService, KeepAlive},
    },
    service::{client::Client, login, tray, Server, ServerOptions},
    session::{self, Config},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn start(home: &std::path::Path) -> crate::service::RunningService {
    let config_home = home.to_path_buf();
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fakes/fake_claude.py");
    let record = home.join("fake-record.json");
    let python = std::env::var("AXIOM_TEST_PYTHON").unwrap_or_else(|_| {
        let names = if cfg!(windows) { ["python", "python3"] } else { ["python3", "python"] };
        names
            .into_iter()
            .find(|name| {
                crate::base::proc::cmd(name)
                    .arg("--version")
                    .output()
                    .is_ok_and(|out| out.status.success())
            })
            .expect("Python not found")
            .into()
    });
    let calls = || Arc::new(Mutex::new(Vec::new()));
    Server::start(ServerOptions {
        home: home.to_path_buf(),
        config: Arc::new(move || Config {
            claude: Ok(vec![
                python.clone(),
                "-I".into(),
                "-c".into(),
                "import os,runpy,sys; os.environ['FAKE_CLAUDE_SCENARIO']='tick'; os.environ['FAKE_CLAUDE_RECORD']=sys.argv[2]; sys.argv=[sys.argv[1]]+sys.argv[3:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(),
                fake.to_string_lossy().into_owned(),
                record.to_string_lossy().into_owned(),
            ]),
            home: config_home.clone(),
            bin: config_home.join("bin"),
            clock: session::Clock::system(),
        }),
        tray: Box::new(tray::Recording::new(calls())),
        desktop: Box::new(tray::Recording::new(calls())),
        login: Box::new(login::Recording::new(calls())),
        login_launch: false,
    })
    .unwrap()
}

#[test]
fn without_a_service_the_window_cannot_keep_wishes_alive() {
    let home = tempfile::tempdir().unwrap();
    let background = ServiceBackground::new(home.path());
    assert!(matches!(background.status(), KeepAlive::Unavailable { .. }));
    assert!(background.detach(&["session".into()]).is_err());
    assert!(background.reattach().is_err());
    assert!(background.quit(true).is_err());
}

#[test]
fn detach_keeps_wishes_running_and_only_quit_stops_the_service() {
    let home = tempfile::tempdir().unwrap();
    let server = start(home.path());
    let background = ServiceBackground::new(home.path());
    assert_eq!(background.status(), KeepAlive::Available);
    assert!(background.reattach().unwrap().is_empty());

    let client = Client::connect(home.path()).unwrap();
    let handle = client
        .start(crate::service::StartRequest {
            wish: "test wish".into(),
            cwd: home.path().to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let session = loop {
        let snapshot = client.snapshot().unwrap();
        if let Some(session) = snapshot
            .wishes
            .iter()
            .find(|w| w.handle == handle)
            .and_then(|w| w.session.clone())
        {
            break session;
        }
        assert!(Instant::now() < deadline, "service did not report a session");
        std::thread::sleep(Duration::from_millis(50));
    };
    let running = vec![session.clone()];

    // Closing the window detaches: the wish stays in the service and is found again.
    background.detach(&running).unwrap();
    assert_eq!(background.reattach().unwrap(), running);
    let wish = client
        .snapshot()
        .unwrap()
        .wishes
        .into_iter()
        .find(|w| w.handle == handle)
        .unwrap();
    assert_eq!(wish.state, "running");

    // Quitting is the explicit action: it stops the service and the running wish.
    background.quit(true).unwrap();
    server.wait();
    assert!(matches!(background.status(), KeepAlive::Unavailable { .. }));
}
