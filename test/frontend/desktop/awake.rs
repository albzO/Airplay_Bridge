use super::*;
#[test]
fn system_and_display_request_is_idempotent_and_closes_its_handle() {
    // 数值句柄可被并行测试复用；独立进程里保留真实 CloseHandle 验证。
    // Isolate the real CloseHandle check from handle-value reuse in concurrent tests.
    if std::env::var_os("AIRPLAY_AWAKE_HANDLE_CHILD").is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "awake::tests::system_and_display_request_is_idempotent_and_closes_its_handle",
                "--nocapture",
            ])
            .env("AIRPLAY_AWAKE_HANDLE_CHILD", "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated awake handle check timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    let mut request = Awake::default();
    request.set(true).unwrap();
    let first = request.handle.unwrap();
    request.set(true).unwrap();
    assert!(request.active());
    assert_eq!(request.handle, Some(first));
    request.set(false).unwrap();
    request.set(false).unwrap();
    assert!(!request.active());
    let mut flags = 0;
    assert!(
        unsafe {
            windows::Win32::Foundation::GetHandleInformation(HANDLE(first as *mut _), &mut flags)
        }
        .is_err()
    );
    request.set(true).unwrap();
    let last = request.handle.unwrap();
    drop(request);
    assert!(
        unsafe {
            windows::Win32::Foundation::GetHandleInformation(HANDLE(last as *mut _), &mut flags)
        }
        .is_err()
    );
}

#[test]
fn activation_acquires_both_types_and_display_failure_rolls_back_system() {
    let mut calls = Vec::new();
    activate(|kind, enabled| {
        calls.push((kind, enabled));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        calls,
        vec![
            (PowerRequestSystemRequired, true),
            (PowerRequestDisplayRequired, true)
        ]
    );
    calls.clear();
    let error = activate(|kind, enabled| {
        calls.push((kind, enabled));
        if kind == PowerRequestDisplayRequired {
            Err("fixture display failure".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(error.contains("fixture display failure"));
    assert_eq!(
        calls,
        vec![
            (PowerRequestSystemRequired, true),
            (PowerRequestDisplayRequired, true),
            (PowerRequestSystemRequired, false)
        ]
    );
}

#[test]
fn system_failure_stops_acquisition_and_rollback_failure_preserves_both_errors() {
    let mut calls = 0;
    let error = activate(|_, _| {
        calls += 1;
        Err("fixture system failure".into())
    })
    .unwrap_err();
    assert_eq!(calls, 1);
    assert!(error.contains("fixture system failure"));
    let error = activate(|kind, enabled| {
        if kind == PowerRequestDisplayRequired {
            Err("fixture display failure".into())
        } else if !enabled {
            Err("fixture rollback failure".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(error.contains("fixture display failure"));
    assert!(error.contains("fixture rollback failure"));
}
