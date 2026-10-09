use super::*;
#[test]
#[ignore = "advertises a DACP service and opens local callback/control listeners"]
fn advertised_callback_forwards_to_separate_control_pipe() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build");
    std::fs::create_dir_all(&directory).unwrap();
    let server = Dacp::start(
        Ipv4Addr::LOCALHOST,
        "A1B2C3D4E5F60999",
        "42",
        &directory.join("dacp-check.log"),
    )
    .unwrap();
    let mut control = TcpStream::connect((Ipv4Addr::LOCALHOST, server.control_port)).unwrap();
    control
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, server.callback_port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client
        .write_all(
            b"GET /ctrl-int/1/setproperty?dmcp.volume=30 HTTP/1.1\r\nActive-Remote: 42\r\n\r\n",
        )
        .unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 204"));
    let mut command = [0; 7];
    control.read_exact(&mut command).unwrap();
    assert_eq!(&command, b"SET 30\n");
    drop(server);
}
#[test]
fn device_report_and_request_have_distinct_units() {
    assert_eq!(
        command("/ctrl-int/1/setproperty?dmcp.device-volume=-12.5&other=1").as_deref(),
        Some("REPORT -12.5\n")
    );
    assert_eq!(
        command("/ctrl-int/1/setproperty?dmcp.volume=50").as_deref(),
        Some("SET 50\n")
    );
    assert_eq!(command("/ctrl-int/1/volumeup").as_deref(), Some("STEP 5\n"));
    assert!(command("/ctrl-int/1/setproperty?dmcp.volume=NaN").is_none());
    assert!(command("/ctrl-int/1/setproperty?dmcp.volume=101").is_none());
    assert!(command("/ctrl-int/1/setproperty?dmcp.device-volume=-145").is_none());
}
#[test]
fn fragmented_callback_validates_active_remote() {
    for active in ["123", "wrong"] {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            let data = format!(
                "GET /ctrl-int/1/setproperty?dmcp.volume=40 HTTP/1.1\r\nActive-Remote: {active}\r\n\r\n"
            );
            for chunk in data.as_bytes().chunks(3) {
                stream.write_all(chunk).unwrap();
            }
            stream
        });
        let (mut stream, _) = listener.accept().unwrap();
        let result = request(&mut stream, "123");
        if active == "123" {
            assert_eq!(result.unwrap().as_deref(), Some("SET 40\n"));
        } else {
            assert_eq!(
                result.unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
        drop(client.join().unwrap());
    }
}
