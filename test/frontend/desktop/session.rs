use super::*;
use crate::{Engine, Settings, auth_memory, awake, configuration::Operation};
use std::{path::PathBuf, sync::mpsc, thread};

fn engine() -> Arc<Engine> {
    Arc::new(Engine {
        root: PathBuf::new(),
        portable: false,
        backend: PathBuf::new(),
        settings: Mutex::new(Settings::default()),
        auth_memory: Mutex::new(auth_memory::Memory::default()),
        session: Mutex::new(None),
        source: Mutex::new(None),
        configuration_busy: AtomicBool::new(false),
        discovering: AtomicBool::new(false),
        quitting: AtomicBool::new(false),
        capture_enabled: AtomicBool::new(false),
        awake: Mutex::new(awake::Awake::default()),
    })
}

#[test]
fn discovery_blocks_streaming_and_itself_but_not_configuration() {
    let e = engine();
    let busy = |e: &Engine| {
        e.configuration_busy.load(Ordering::Acquire) || e.discovering.load(Ordering::Acquire)
    };
    let job = e.prepare_discover_devices().unwrap();
    assert!(reserve(&mut e.session.lock().unwrap(), busy(&e), [0, 1]).is_err());
    assert!(e.prepare_discover_devices().is_err());
    // 发现期间仍可保存设置、切换采集。
    // Settings and capture can still change during discovery.
    drop(Operation::begin(e.clone(), false).unwrap());
    // 未运行就丢弃的作业也要释放占用。
    // A job dropped without running still releases its claim.
    drop(job);
    let (id, _, preparation) = reserve(&mut e.session.lock().unwrap(), busy(&e), [0, 1]).unwrap();
    assert!(e.prepare_discover_devices().is_err());
    preparation.finish();
    release(&mut e.session.lock().unwrap(), id);
    assert!(e.prepare_discover_devices().is_ok());
}

fn awake(e: &Engine) -> bool {
    e.awake.lock().unwrap().active()
}

#[test]
fn awake_request_follows_the_established_stream_only() {
    let e = engine();
    e.sync_awake().unwrap();
    assert!(!awake(&e), "idle app must not hold a request");
    let (id, control, preparation) =
        reserve(&mut e.session.lock().unwrap(), false, [0, 1]).unwrap();
    e.sync_awake().unwrap();
    assert!(!awake(&e), "preparing is not streaming");
    preparation.activate(&control).unwrap();
    e.sync_awake().unwrap();
    assert!(awake(&e));
    // 串流中切换开关立即生效。
    // Toggling the switch during a stream applies at once.
    e.apply_awake(false).unwrap();
    assert!(!awake(&e));
    e.apply_awake(true).unwrap();
    assert!(awake(&e));
    preparation.finish();
    release(&mut e.session.lock().unwrap(), id);
    e.sync_awake().unwrap();
    assert!(!awake(&e));
    // 已结束的串流不能被迟到的设置保存重新唤醒。
    // A late settings save cannot reacquire the request after the stream ended.
    e.apply_awake(true).unwrap();
    assert!(!awake(&e));
}

#[test]
fn awake_request_is_released_when_cleanup_times_out_or_the_app_quits() {
    let e = engine();
    let (id, control, preparation) =
        reserve(&mut e.session.lock().unwrap(), false, [0, 1]).unwrap();
    preparation.activate(&control).unwrap();
    e.sync_awake().unwrap();
    assert!(awake(&e));
    e.quitting.store(true, Ordering::Release);
    e.sync_awake().unwrap();
    assert!(!awake(&e));
    e.quitting.store(false, Ordering::Release);
    e.sync_awake().unwrap();
    assert!(awake(&e));
    control.cleanup_complete.store(false, Ordering::Release);
    preparation.finish();
    release(&mut e.session.lock().unwrap(), id);
    e.sync_awake().unwrap();
    assert!(
        e.session.lock().unwrap().is_some(),
        "reservation is retained"
    );
    assert!(!awake(&e));
}

#[test]
fn saving_while_idle_never_acquires_and_a_failed_save_keeps_the_stream_awake() {
    let root = std::env::temp_dir().join(format!("airplay-awake-save-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut e = engine();
    Arc::get_mut(&mut e).unwrap().root = root.clone();
    let idle = e.prepare_save_settings(Settings::default()).unwrap()();
    let idle_awake = awake(&e);
    let (_, control, preparation) = reserve(&mut e.session.lock().unwrap(), false, [0, 1]).unwrap();
    preparation.activate(&control).unwrap();
    e.sync_awake().unwrap();
    // 暂存文件路径被目录占用，触发真实写入失败。
    // A directory at the pending-file path forces a real write failure.
    std::fs::create_dir(root.join("settings.pending.json")).unwrap();
    let off = Settings {
        keep_awake: false,
        ..Settings::default()
    };
    let failed = e.prepare_save_settings(off.clone()).unwrap()();
    let after_failure = awake(&e);
    std::fs::remove_dir(root.join("settings.pending.json")).unwrap();
    let saved = e.prepare_save_settings(off).unwrap()();
    let after_save = awake(&e);
    let _ = std::fs::remove_dir_all(&root);
    assert!(idle.is_ok() && !idle_awake);
    assert!(failed.is_err() && after_failure);
    assert!(saved.is_ok() && !after_save);
}

#[test]
fn stopped_blocked_preparation_does_not_hold_session_lock_or_publish_late_result() {
    let e = engine();
    let (id, control, preparation) =
        reserve(&mut e.session.lock().unwrap(), false, [0, 1]).unwrap();
    let (entered, ready) = mpsc::channel();
    let (resume, blocked) = mpsc::channel();
    let worker_control = control.clone();
    let worker_preparation = preparation.clone();
    // 与生产枚举相同的步骤边界，替换驱动调用为可控阻塞。
    // Use the production enumeration step boundary with a controlled blocking driver substitute.
    let worker = thread::spawn(move || {
        worker_preparation.step(&worker_control, || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
            Ok("late enumeration")
        })
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    let stop_engine = e.clone();
    let (sent, received) = mpsc::channel();
    let stopper = thread::spawn(move || {
        stop(&stop_engine.session.lock().unwrap(), id);
        sent.send(()).unwrap();
    });
    // 先解除替身阻塞，再断言，避免测试失败遗留后台线程。
    // Unblock before asserting so a failed test cannot leave the substitute worker behind.
    let stopped = received.recv_timeout(Duration::from_secs(2));
    resume.send(()).unwrap();
    stopper.join().unwrap();
    let result = worker.join().unwrap();
    assert!(stopped.is_ok());
    assert!(result.unwrap_err().contains("已取消"));
    assert!(preparation.activate(&control).is_err());
    assert!(Operation::begin(e.clone(), true).is_err());
    assert!(preparation.finish());
    release(&mut e.session.lock().unwrap(), id);
    assert!(e.session.lock().unwrap().is_none());
}

#[test]
fn preparation_timeout_retains_occupancy_and_notifies_only_once() {
    let mut active = None;
    let (id, control, preparation) = reserve(&mut active, false, [0, 1]).unwrap();
    let error = preparation
        .expire(&control, Instant::now() + PREPARE_TIMEOUT)
        .unwrap();
    assert!(error.contains("START_TIMEOUT"));
    assert!(control.stop.load(Ordering::Acquire));
    assert!(!control.cleanup_complete.load(Ordering::Acquire));
    assert!(preparation.claim_terminal());
    assert!(preparation.activate(&control).is_err());
    assert!(!preparation.finish());
    release(&mut active, id);
    assert!(reserve(&mut active, false, [0, 1]).is_err());
    assert_eq!(active.as_ref().unwrap().id, id);
}

#[test]
fn stop_deadline_is_observed_before_prepare_deadline_and_active_is_not_expired() {
    let mut active = None;
    let (_, control, preparation) = reserve(&mut active, false, [0, 1]).unwrap();
    let now = Instant::now();
    control.request_stop_until(now);
    assert!(
        preparation
            .expire(&control, now)
            .unwrap()
            .contains("STOP_TIMEOUT")
    );
    let fresh = Preparation::new(now + PREPARE_TIMEOUT);
    let control = GuiControl::new([0, 1]);
    fresh.activate(&control).unwrap();
    assert!(fresh.expire(&control, now + PREPARE_TIMEOUT).is_none());
    assert!(fresh.finish());
    assert!(!fresh.finish());
}

#[test]
fn old_stop_and_release_cannot_touch_a_new_reservation() {
    let mut active = None;
    let (old, _, prep) = reserve(&mut active, false, [0, 1]).unwrap();
    prep.finish();
    release(&mut active, old);
    let (new, control, _) = reserve(&mut active, false, [0, 1]).unwrap();
    assert!(new > old);
    stop(&active, old);
    release(&mut active, old);
    assert_eq!(active.as_ref().unwrap().id, new);
    assert!(!control.stop.load(Ordering::Acquire));
}

#[test]
fn configuration_discovery_and_preview_share_admission_and_reject_late_session_work() {
    let e = engine();
    let operation = Operation::begin(e.clone(), false).unwrap();
    assert!(
        reserve(
            &mut e.session.lock().unwrap(),
            e.configuration_busy.load(Ordering::Acquire),
            [0, 1]
        )
        .is_err()
    );
    assert!(Operation::begin(e.clone(), true).is_err());
    drop(operation);
    let (id, control, preparation) =
        reserve(&mut e.session.lock().unwrap(), false, [0, 1]).unwrap();
    assert!(Operation::begin(e.clone(), true).is_err());
    preparation.activate(&control).unwrap();
    let operation = Operation::begin(e.clone(), true).unwrap();
    assert!(Operation::begin(e.clone(), false).is_err());
    stop(&e.session.lock().unwrap(), id);
    assert!(operation.check().is_err());
    preparation.finish();
    release(&mut e.session.lock().unwrap(), id);
    assert!(operation.check().is_err());
    assert!(
        reserve(
            &mut e.session.lock().unwrap(),
            e.configuration_busy.load(Ordering::Acquire),
            [0, 1]
        )
        .is_err()
    );
    drop(operation);
    let operation = Operation::begin(e.clone(), false).unwrap();
    e.quitting.store(true, Ordering::Release);
    assert!(operation.check().is_err());
    drop(operation);
    assert!(Operation::begin(e.clone(), false).is_err());
}

#[test]
fn cancel_after_pending_write_preserves_committed_settings_and_rejects_late_commit() {
    use std::{fs, sync::atomic::AtomicUsize};
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("airplay-preparation-save-{}", std::process::id())),
    );
    fs::create_dir(&directory.0).unwrap();
    Settings::default().save(&directory.0).unwrap();
    let mut e = engine();
    Arc::get_mut(&mut e).unwrap().root = directory.0.clone();
    let (id, control, preparation) =
        reserve(&mut e.session.lock().unwrap(), false, [0, 1]).unwrap();
    let (entered, pending) = mpsc::channel();
    let (resume, blocked) = mpsc::channel();
    let worker_engine = e.clone();
    let worker_preparation = preparation.clone();
    let worker = thread::spawn(move || {
        let next = Settings {
            latency: 600,
            ..Settings::default()
        };
        let calls = AtomicUsize::new(0);
        let result = next.save_checked(&worker_engine.root, || {
            if calls.fetch_add(1, Ordering::Relaxed) == 1 {
                entered.send(()).unwrap();
                // 真实暂存文件已经写入，在提交前阻塞以便发送取消。
                // The callback blocks after a real pending-file write, not a mocked successful save.
                blocked.recv().unwrap();
            }
            worker_preparation.check(&control)
        });
        if result.is_ok() {
            *worker_engine.settings.lock().unwrap() = next;
        }
        result
    });
    pending.recv_timeout(Duration::from_secs(2)).unwrap();
    let pending_exists = directory.0.join("settings.pending.json").is_file();
    stop(&e.session.lock().unwrap(), id);
    resume.send(()).unwrap();
    let result = worker.join().unwrap();
    assert!(pending_exists);
    assert!(result.unwrap_err().contains("已取消"));
    assert_eq!(Settings::load(&directory.0).latency, 300);
    assert_eq!(e.settings.lock().unwrap().latency, 300);
    assert!(!directory.0.join("settings.pending.json").exists());
    preparation.finish();
    release(&mut e.session.lock().unwrap(), id);
    assert!(e.session.lock().unwrap().is_none());
}
