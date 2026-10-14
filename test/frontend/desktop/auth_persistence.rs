use super::*;
use serde_json::json;
use std::{sync::mpsc, time::Duration};

fn targets() -> Vec<(String, String)> {
    vec![
        ("left".into(), "left|key".into()),
        ("right".into(), "right|key".into()),
    ]
}
fn password(host: &str) -> Value {
    json!({"kind":"password_required","host":host})
}

#[test]
fn slow_disk_does_not_block_observation_and_repeated_evidence_is_coalesced() {
    let control = Arc::new(GuiControl::new([0, 1]));
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let saved = calls.clone();
    let mut worker = Worker::start(targets(), control.clone(), move |identity| {
        if identity == "left|key" {
            entered.send(()).unwrap();
            let _ = gate.recv_timeout(Duration::from_secs(5));
        }
        saved.lock().unwrap().push(identity.to_owned());
        Ok(())
    })
    .unwrap();
    let sender = worker.sender();
    sender.observe(&password("left"));
    began.recv_timeout(Duration::from_secs(2)).unwrap();
    for _ in 0..1000 {
        sender.observe(&password("left"));
        sender.observe(&password("right"));
        sender.observe(&password("unknown"));
        sender.observe(
            &json!({"kind":"native","line":"[PROBE] ERROR code=PAIRING_BACKOFF host=right"}),
        );
    }
    let queue = sender.pending.0.lock().unwrap();
    assert_eq!(queue.requests.len(), 1);
    assert_eq!(queue.seen, 3);
    drop(queue);
    assert!(sender.take_errors().is_empty());
    release.send(()).unwrap();
    worker
        .finish_until(Instant::now() + Duration::from_secs(2))
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), ["left|key", "right|key"]);
    sender.observe(&password("left"));
    assert!(sender.pending.0.lock().unwrap().requests.is_empty());
}

#[test]
fn failures_and_panics_produce_bounded_errors_without_touching_gui_callbacks() {
    let mut worker = Worker::start(targets(), Arc::new(GuiControl::new([0, 1])), |identity| {
        if identity == "right|key" {
            panic!("fixture save panic");
        }
        Err("fixture disk error ".repeat(1000))
    })
    .unwrap();
    let sender = worker.sender();
    sender.observe(&password("left"));
    sender.observe(&password("right"));
    worker
        .finish_until(Instant::now() + Duration::from_secs(2))
        .unwrap();
    let errors = sender.take_errors();
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().all(|error| error.chars().count() <= 2048));
    assert!(errors[1].contains("后台保存异常"));
    assert!(sender.take_errors().is_empty());
}

#[test]
fn timeout_retains_session_and_does_not_start_more_disk_work_after_deadline() {
    let mut active = None;
    let (id, control, _) = crate::session::reserve(&mut active, false, [0, 1]).unwrap();
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let saved = calls.clone();
    let mut worker = Worker::start(targets(), control.clone(), move |identity| {
        saved.lock().unwrap().push(identity.to_owned());
        entered.send(()).unwrap();
        let _ = gate.recv_timeout(Duration::from_secs(5));
        Ok(())
    })
    .unwrap();
    let sender = worker.sender();
    sender.observe(&password("left"));
    began.recv_timeout(Duration::from_secs(2)).unwrap();
    sender.observe(&password("right"));
    let deadline = control
        .shutdown
        .begin_until(Instant::now() + Duration::from_millis(30));
    assert!(
        worker
            .finish_until(deadline)
            .unwrap_err()
            .contains("STOP_TIMEOUT")
    );
    assert!(worker.unfinished());
    active.as_mut().unwrap().auth_policy = Some(worker);
    crate::session::release(&mut active, id);
    assert!(active.is_some());
    assert!(crate::session::reserve(&mut active, false, [0, 1]).is_err());
    assert_eq!(control.shutdown.begin(), deadline);
    release.send(()).unwrap();
    let worker = active.as_mut().unwrap().auth_policy.as_mut().unwrap();
    // Collect the actual handle after releasing the fixture; never renew the production deadline.
    homepod_test::shutdown::join_until(
        &mut worker.thread,
        Instant::now() + Duration::from_secs(2),
        false,
    )
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), ["left|key"]);
    assert!(sender.take_errors()[0].contains("期限已到"));
}
