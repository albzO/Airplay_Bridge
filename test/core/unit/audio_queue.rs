use super::*;

#[test]
fn duration_bound_is_independent_of_rate_and_includes_in_flight_work() {
    for rate in [8000, 44100, 48000, 96000, 192000] {
        let (tx, rx) = channel();
        for _ in 0..64 {
            tx.try_send(rate as usize / 100, rate, || 1).unwrap();
        }
        let active = rx.recv().unwrap();
        assert_eq!(tx.stats.pending_ms(), 640.0);
        let rejected = tx.try_send(rate as usize / 100, rate, || {
            panic!("full budget must skip allocation")
        });
        assert_eq!(rejected, Err(QueueError::Full));
        drop(active);
        tx.try_send(rate as usize / 100, rate, || 2).unwrap();
        drop(rx);
        assert_eq!(tx.stats.pending_ms(), 0.0);
        assert_eq!(tx.stats.peak_ms(), 640.0);
    }
}

#[test]
fn variable_blocks_hard_cap_disconnect_and_invalid_format_release_reservations() {
    let (tx, rx) = channel();
    for frames in [480, 960, 240, 120] {
        tx.try_send(frames, 48000, || frames).unwrap();
    }
    assert_eq!(tx.stats.pending_ms(), 37.5);
    while let Ok(block) = rx.try_recv() {
        drop(block);
    }
    for _ in 0..BLOCK_LIMIT {
        tx.try_send(1, 192000, || 1).unwrap();
    }
    let pending = tx.stats.pending_ms();
    assert_eq!(tx.try_send(1, 192000, || 1), Err(QueueError::Full));
    assert_eq!(tx.stats.pending_ms(), pending);
    drop(rx);
    assert_eq!(tx.stats.pending_ms(), 0.0);
    assert_eq!(tx.try_send(480, 48000, || 1), Err(QueueError::Disconnected));
    assert_eq!(tx.stats.pending_ms(), 0.0);
    assert_eq!(tx.try_send(0, 48000, || 1), Err(QueueError::InvalidFormat));
    assert_eq!(tx.try_send(480, 0, || 1), Err(QueueError::InvalidFormat));
    assert_eq!(tx.try_send(48000, 48000, || 1), Err(QueueError::Full));
}
