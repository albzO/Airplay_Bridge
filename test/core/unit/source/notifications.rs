use super::*;

#[test]
fn slow_gui_coalesces_levels_and_preserves_control_order() {
    let producer = thread::current().id();
    let (seen, events) = mpsc::channel();
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let mut worker = Worker::start(Arc::new(move |event| {
        assert_ne!(thread::current().id(), producer);
        let first = event == json!("blocked");
        seen.send(event).unwrap();
        if first {
            gate.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
        }
    }));
    let sender = worker.sender();
    sender.level(json!("blocked"));
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)).unwrap(),
        json!("blocked")
    );
    for level in 0..1000 {
        sender.level(json!(level));
    }
    sender.control(json!("warning-1")).unwrap();
    sender.control(json!("warning-2")).unwrap();
    assert_eq!(sender.0.0.lock().unwrap().level, Some(json!(999)));
    release.send(()).unwrap();
    for expected in [json!("warning-1"), json!("warning-2"), json!(999)] {
        assert_eq!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            expected
        );
    }
    sender.finish(json!({"peaks":[0,0],"error":null}));
    assert_eq!(
        events.recv_timeout(Duration::from_secs(2)).unwrap()["peaks"],
        json!([0, 0])
    );
    worker
        .join_until(Instant::now() + Duration::from_secs(2))
        .unwrap();
}

#[test]
fn full_control_queue_keeps_terminal_slot_and_discards_stale_level() {
    let sender = Sender::default();
    for i in 0..CONTROL_CAPACITY {
        sender.control(json!(i)).unwrap();
    }
    assert!(
        sender
            .control(json!("overflow"))
            .unwrap_err()
            .contains("SOURCE_NOTIFY_FAILED")
    );
    sender.level(json!("stale"));
    sender.finish(json!("terminal"));
    sender.level(json!("too late"));
    let pending = sender.0.0.lock().unwrap();
    assert_eq!(pending.controls.len(), CONTROL_CAPACITY);
    assert_eq!(pending.terminal, Some(json!("terminal")));
    assert!(pending.level.is_none());
    assert!(pending.closed);
}
