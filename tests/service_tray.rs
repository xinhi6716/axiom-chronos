use axiom::{
    base::{Failure, Res},
    service::{
        self,
        login::{self, LoginStart},
        tray::{self, Desktop, Tray},
        RunningService, Server, ServerOptions,
    },
    session,
};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

fn start_service(home: &Path, desktop_calls: Arc<Mutex<Vec<String>>>) -> RunningService {
    let cfg_home = home.to_path_buf();
    Server::start(ServerOptions {
        home: home.to_path_buf(),
        config: Arc::new(move || {
            let mut config = session::Config::from_env();
            config.home = cfg_home.clone();
            config
        }),
        tray: Box::new(tray::Recording::default()),
        desktop: Box::new(tray::Recording::new(desktop_calls)),
        login: Box::new(login::Recording::default()),
        login_launch: false,
    })
    .unwrap()
}

fn service_exit(running: RunningService) -> mpsc::Receiver<i32> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(running.wait());
    });
    rx
}

#[test]
fn forwarded_tray_actions_reach_service() {
    let dir = tempfile::tempdir().unwrap();
    let desktop_calls = Arc::new(Mutex::new(Vec::new()));
    let running = start_service(dir.path(), desktop_calls.clone());
    let (tx, rx) = mpsc::channel();
    let forwarder = service::forward_tray_actions(rx, running.tray_actions());
    let exit = service_exit(running);

    tx.send(tray::TrayAction::Open).unwrap();
    tx.send(tray::TrayAction::PauseAll).unwrap();
    tx.send(tray::TrayAction::Quit).unwrap();
    drop(tx);

    assert_eq!(exit.recv_timeout(Duration::from_secs(3)).unwrap(), 0);
    assert_eq!(*desktop_calls.lock().unwrap(), vec!["open"]);
    forwarder.join().unwrap();
}

#[test]
fn unforwarded_tray_quit_does_not_stop_service() {
    let dir = tempfile::tempdir().unwrap();
    let running = start_service(dir.path(), Arc::new(Mutex::new(Vec::new())));
    let (tx, _rx) = mpsc::channel();

    tx.send(tray::TrayAction::Quit).unwrap();
    thread::sleep(Duration::from_millis(200));
    let still_running = matches!(
        service::lock::acquire(dir.path()),
        Err(e) if e.kind == "already_running"
    );
    running.quit(true).unwrap();

    assert_eq!(
        service_exit(running)
            .recv_timeout(Duration::from_secs(3))
            .unwrap(),
        0
    );
    assert!(still_running, "unforwarded Quit stopped the service");
}

#[test]
fn windows_login_commands_are_exact() {
    let command = strings(&[r"C:\Program Files\Axiom\axiom.exe", "serve", "--login"]);
    let add = strings(&[
        "reg.exe",
        "add",
        login::RUN_KEY,
        "/v",
        "Axiom Chronos",
        "/t",
        "REG_SZ",
        "/d",
        r#""C:\Program Files\Axiom\axiom.exe" "serve" "--login""#,
        "/f",
    ]);
    let delete = strings(&[
        "reg.exe",
        "delete",
        login::RUN_KEY,
        "/v",
        "Axiom Chronos",
        "/f",
    ]);
    assert_eq!(
        login::windows_commands(&command),
        (add.clone(), delete.clone())
    );
    assert_eq!(login::quote_arg("a\"b"), "\"a\\\"b\"");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let adapter = login::Windows::new(Box::new(login::RecordingRunner::new(calls.clone())));
    adapter.enable(&command).unwrap();
    adapter.disable().unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![add, delete]);
    assert_eq!(adapter.enable(&[]).unwrap_err().kind, "invalid_argument");
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[test]
fn xdg_autostart_written_under_temp_config() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = login::Xdg::new(dir.path().join("autostart"));
    let command = strings(&["/tmp/my app", "serve", "x\\y\"$`%"]);
    adapter.enable(&command).unwrap();
    let contents =
        std::fs::read_to_string(dir.path().join("autostart/axiom-chronos.desktop")).unwrap();
    assert_eq!(contents, login::xdg_entry(&command));
    assert_eq!(contents, "[Desktop Entry]\nType=Application\nName=Axiom Chronos\nExec=\"/tmp/my app\" serve \"x\\\\y\\\"\\$\\`%%\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n");
    assert_eq!(adapter.enable(&[]).unwrap_err().kind, "invalid_argument");
}

#[test]
fn disable_removes_entry() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = login::Xdg::new(dir.path().join("autostart"));
    adapter.enable(&strings(&["axiom", "serve"])).unwrap();
    adapter.disable().unwrap();
    assert!(!dir.path().join("autostart/axiom-chronos.desktop").exists());
    adapter.disable().unwrap();
}

#[test]
fn status_text_counts() {
    for (n, expected) in [
        (0, "Chronos: idle"),
        (1, "Chronos: 1 wish running"),
        (2, "Chronos: 2 wishes running"),
        (11, "Chronos: 11 wishes running"),
    ] {
        assert_eq!(tray::status_text(n), expected);
    }
    let mut recording = tray::Recording::default();
    recording.update(&tray::TrayStatus {
        running: 2,
        text: "custom".into(),
    });
    assert_eq!(*recording.calls.lock().unwrap(), vec!["custom"]);
}

type SpawnCalls = Arc<Mutex<Vec<(PathBuf, Vec<String>)>>>;
struct Recorder(SpawnCalls);
impl tray::Spawner for Recorder {
    fn spawn(&self, exe: &Path, args: &[String]) -> Res<()> {
        self.0
            .lock()
            .unwrap()
            .push((exe.to_path_buf(), args.to_vec()));
        Ok(())
    }
}
#[test]
fn desktop_launcher_without_app_reports_no_desktop() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let launcher = tray::Launcher::new(None, Box::new(Recorder(calls.clone())));
    assert_eq!(launcher.open().unwrap_err().kind, "no_desktop");
    assert_eq!(launcher.confirm_quit(2).unwrap_err().kind, "no_desktop");
    let path = PathBuf::from("app");
    let launcher = tray::Launcher::new(Some(path.clone()), Box::new(Recorder(calls.clone())));
    launcher.open().unwrap();
    launcher.confirm_quit(2).unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            (path, vec![]),
            (PathBuf::from("app"), strings(&["--confirm-quit"]))
        ]
    );
    let dir = tempfile::tempdir().unwrap();
    let env = dir.path().join("environment");
    let current = dir.path().join("axiom.exe");
    let sibling = dir.path().join(if cfg!(windows) {
        "chronos.exe"
    } else {
        "chronos"
    });
    std::fs::write(&env, b"").unwrap();
    std::fs::write(&sibling, b"").unwrap();
    assert_eq!(tray::locate_app(env.to_str(), &current), Some(env));
    assert_eq!(
        tray::locate_app(Some("missing"), &current),
        Some(sibling.clone())
    );
    std::fs::remove_file(&sibling).unwrap();
    assert_eq!(tray::locate_app(None, &sibling), Some(sibling));
    assert_eq!(tray::locate_app(None, &current), None);
}

struct FailingRunner;
impl login::Runner for FailingRunner {
    fn run(&self, _: &[String]) -> Res<()> {
        Err(Failure::new("test_failure", "failed"))
    }
}
#[test]
fn windows_runner_failure_negative_control() {
    let adapter = login::Windows::new(Box::new(FailingRunner));
    assert_eq!(
        adapter.enable(&strings(&["axiom"])).unwrap_err().kind,
        "test_failure"
    );
    adapter.disable().unwrap();
}
