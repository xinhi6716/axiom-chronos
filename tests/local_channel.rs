use axiom::base::ipc::{endpoint_for, read_json, write_json, Conn, Endpoint, Listener};
use serde_json::json;
use std::{
    io::{BufReader, Read, Write},
    time::Duration,
};
const TIMEOUT: Duration = Duration::from_secs(2);
fn setup() -> (tempfile::TempDir, Endpoint) {
    let dir = tempfile::tempdir().unwrap();
    let ep = endpoint_for(dir.path()).unwrap();
    (dir, ep)
}
fn pair() -> (tempfile::TempDir, Listener, Conn, Conn) {
    let (dir, ep) = setup();
    let listener = Listener::bind(&ep).unwrap();
    let client = Conn::connect(&ep, TIMEOUT).unwrap();
    let server = listener.accept_timeout(TIMEOUT).unwrap().unwrap();
    (dir, listener, client, server)
}
#[test]
fn endpoint_is_stable_and_home_derived() {
    let (dir, ep) = setup();
    let (_other, other) = setup();
    assert_eq!(ep.name, endpoint_for(dir.path()).unwrap().name);
    assert_eq!(ep.home, dir.path().canonicalize().unwrap());
    assert_ne!(ep.name, other.name);
    #[cfg(windows)]
    {
        assert!(ep.name.starts_with(r"\\.\pipe\axiom-chronos-"));
        let suffix = ep.name.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 16);
        assert!(suffix.bytes().all(|b| b.is_ascii_hexdigit()));
    }
    #[cfg(unix)]
    {
        assert!(ep.name.len() <= 100);
        let long_home = dir.path().join("x".repeat(110));
        std::fs::create_dir(&long_home).unwrap();
        let long_ep = endpoint_for(&long_home).unwrap();
        assert!(long_ep.name.len() <= 100);
        assert_ne!(ep.name, long_ep.name);
        let _listener = Listener::bind(&long_ep).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let path = std::path::Path::new(&long_ep.name);
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn bind_twice_is_in_use() {
    let (_dir, ep) = setup();
    let _listener = Listener::bind(&ep).unwrap();
    assert_eq!(Listener::bind(&ep).err().unwrap().kind, "in_use");
}
#[test]
fn connect_without_listener_is_unavailable() {
    let (_dir, ep) = setup();
    assert_eq!(
        Conn::connect(&ep, Duration::ZERO).err().unwrap().kind,
        "unavailable"
    );
}
#[test]
fn roundtrip_json_both_directions() {
    let (_dir, _listener, mut client, mut server) = pair();
    write_json(&mut client, &json!({"request": 7})).unwrap();
    assert_eq!(
        read_json(&mut BufReader::new(server.try_clone().unwrap()), 1024).unwrap(),
        Some(json!({"request": 7}))
    );
    write_json(&mut server, &json!({"response": true})).unwrap();
    assert_eq!(
        read_json(&mut BufReader::new(client), 1024).unwrap(),
        Some(json!({"response": true}))
    );
}
#[test]
fn clone_reads_while_writing() {
    let (_dir, _listener, client, mut server) = pair();
    let mut clone = client.try_clone().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut b = [0];
        started_tx.send(()).unwrap();
        let result = clone.read(&mut b);
        tx.send((result, b)).unwrap();
    });
    started_rx.recv_timeout(TIMEOUT).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let mut writer = client.try_clone().unwrap();
    let (written_tx, written_rx) = std::sync::mpsc::channel();
    let write_thread = std::thread::spawn(move || {
        let _ = written_tx.send(writer.write_all(b"a"));
    });
    let written = written_rx.recv_timeout(TIMEOUT);
    if written.is_err() {
        client.shutdown();
    }
    written.unwrap().unwrap();
    let mut b = [0];
    server.read_exact(&mut b).unwrap();
    assert_eq!(b, *b"a");
    server.write_all(b"b").unwrap();
    let (result, b) = rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(result.unwrap(), 1);
    assert_eq!(b, *b"b");
    thread.join().unwrap();
    write_thread.join().unwrap();
}
#[test]
fn oversize_line_is_too_large() {
    let (_dir, _listener, mut client, server) = pair();
    client.write_all(b"12345\n").unwrap();
    assert_eq!(
        read_json(&mut BufReader::new(server), 4).unwrap_err().kind,
        "too_large"
    );
}
#[test]
fn eof_returns_none() {
    let (_dir, _listener, client, server) = pair();
    drop(client);
    assert_eq!(read_json(&mut BufReader::new(server), 10).unwrap(), None);
}
#[test]
fn rebind_after_drop_succeeds() {
    let (_dir, ep) = setup();
    let listener = Listener::bind(&ep).unwrap();
    let _client = Conn::connect(&ep, TIMEOUT).unwrap();
    let _server = listener.accept_timeout(TIMEOUT).unwrap().unwrap();
    drop(listener);
    let _listener = Listener::bind(&ep).unwrap();
}
#[cfg(unix)]
#[test]
fn stale_socket_file_is_replaced() {
    let (_dir, ep) = setup();
    std::fs::create_dir_all(std::path::Path::new(&ep.name).parent().unwrap()).unwrap();
    drop(std::os::unix::net::UnixListener::bind(&ep.name).unwrap());
    let _listener = Listener::bind(&ep).unwrap();
}
#[test]
fn shutdown_unblocks_reader() {
    let (_dir, _listener, client, _server) = pair();
    let mut clone = client.try_clone().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut b = [0];
        tx.send(clone.read(&mut b)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(30));
    client.shutdown();
    let result = rx.recv_timeout(TIMEOUT).unwrap();
    assert!(result.is_err() || result.unwrap() == 0);
    thread.join().unwrap();
}
#[test]
fn many_sequential_clients() {
    let (_dir, ep) = setup();
    let listener = Listener::bind(&ep).unwrap();
    for i in 0..30 {
        let mut client = Conn::connect(&ep, TIMEOUT).unwrap();
        let mut server = listener.accept_timeout(TIMEOUT).unwrap().unwrap();
        client.write_all(&[i]).unwrap();
        let mut b = [0];
        server.read_exact(&mut b).unwrap();
        assert_eq!(b, [i]);
    }
}
