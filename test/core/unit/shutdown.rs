use super::*;

#[test]
fn repeated_stop_requests_never_extend_the_budget() {
    let budget = Budget::default();
    let first = budget.begin();
    assert_eq!(budget.begin_until(first + Duration::from_secs(100)), first);
    let earlier = first - Duration::from_secs(1);
    assert_eq!(budget.begin_until(earlier), earlier);
    assert_eq!(budget.clone().current(), Some(earlier));
}

#[test]
fn blocked_workers_share_one_deadline_and_can_be_rejoined_after_release() {
    let mut releases = Vec::new();
    let mut workers = Vec::new();
    for _ in 0..3 {
        let (tx, rx) = mpsc::channel::<()>();
        releases.push(tx);
        workers.push(Some(thread::spawn(move || {
            let _ = rx.recv();
        })));
    }
    let started = Instant::now();
    let deadline = started + Duration::from_millis(40);
    for worker in &mut workers {
        assert!(join_until(worker, deadline, false).is_err());
    }
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(workers.iter().all(Option::is_some));
    drop(releases);
    for worker in &mut workers {
        join_until(worker, Instant::now() + Duration::from_secs(2), false).unwrap();
    }
}

#[test]
fn best_effort_work_does_not_wait_for_a_stalled_callback() {
    let (release, gate) = mpsc::channel::<()>();
    let (exited, done) = mpsc::channel::<()>();
    let start = Instant::now();
    let result = run_until(start + Duration::from_millis(40), move || {
        let _ = gate.recv();
        let _ = exited.send(());
    });
    assert!(result.unwrap_err().contains("STOP_TIMEOUT"));
    assert!(start.elapsed() < Duration::from_millis(500));
    drop(release);
    done.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn expired_deadline_does_not_start_best_effort_work() {
    let (tx, rx) = mpsc::channel::<()>();
    assert!(run_until(Instant::now(), move || tx.send(())).is_err());
    assert!(rx.recv().is_err());
}
