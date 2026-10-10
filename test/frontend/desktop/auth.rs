use super::*;
#[test]
fn stalled_password_prompt_does_not_add_a_fresh_shutdown_timeout() {
    let stop = Arc::new(AtomicBool::new(false));
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let emit: GuiEmitter = Arc::new(move |_| {
        let _ = entered.send(());
        let _ = gate.lock().unwrap().recv();
    });
    let name = format!(
        "\\\\.\\pipe\\airplay-stop-budget-test-{}",
        std::process::id()
    );
    let mut server = Server::start(name.clone(), stop, vec!["127.0.0.1".into()], emit).unwrap();
    let mut client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)
        .unwrap();
    client.write_all(&9u32.to_le_bytes()).unwrap();
    client.write_all(b"127.0.0.1").unwrap();
    began.recv_timeout(Duration::from_secs(3)).unwrap();
    let start = std::time::Instant::now();
    assert!(!server.shutdown_until(start + Duration::from_millis(40)));
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(release);
    drop(client);
    assert!(server.shutdown_until(std::time::Instant::now() + Duration::from_secs(2)));
}
#[test]
fn local_password_roundtrip_and_cancel() {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let emit: GuiEmitter = Arc::new(move |v| {
        tx.send(v).unwrap();
    });
    let name = format!("\\\\.\\pipe\\airplay-bridge-test-{}", std::process::id());
    let server = Server::start(name.clone(), stop, vec!["127.0.0.1".into()], emit).unwrap();
    fn client(name: String) -> thread::JoinHandle<Vec<u8>> {
        thread::spawn(move || {
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(name)
                .unwrap();
            f.write_all(&9u32.to_le_bytes()).unwrap();
            f.write_all(b"127.0.0.1").unwrap();
            let mut size = [0; 4];
            f.read_exact(&mut size).unwrap();
            let mut value = vec![0; u32::from_le_bytes(size) as usize];
            f.read_exact(&mut value).unwrap();
            value
        })
    }
    let c = client(name.clone());
    let event = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(event["host"], "127.0.0.1");
    assert!(event.get("password").is_none());
    server
        .replies
        .send(Reply {
            host: "127.0.0.1".into(),
            password: "local-test-only".into(),
        })
        .unwrap();
    assert_eq!(c.join().unwrap(), b"local-test-only");
    thread::sleep(Duration::from_millis(30));
    let c = client(name);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(3)).unwrap()["kind"],
        "password_required"
    );
    drop(server);
    assert!(c.join().unwrap().is_empty());
}
