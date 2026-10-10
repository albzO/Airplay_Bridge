use super::*;

fn plan(
    timeline: &mut Timeline,
    frames: u64,
    position: u64,
    available: u32,
    ms: u64,
    now: Instant,
) -> Result<PacketPlan> {
    timeline.plan(
        frames,
        position,
        available,
        ms * 10_000,
        false,
        Duration::from_millis(ms),
        now,
    )
}

#[test]
fn idle_silence_has_delivery_margin_block_limit_and_target_cap() {
    let timeline = Timeline::new(48000, 1000, true, 0);
    assert_eq!(timeline.idle_target(Duration::from_millis(39)), 0);
    assert_eq!(
        timeline.idle_frames(0, Duration::from_secs(1), Duration::from_millis(39)),
        0
    );
    assert_eq!(
        timeline.idle_frames(0, Duration::from_secs(1), Duration::from_secs(1)),
        480
    );
    assert_eq!(
        timeline.idle_frames(950, Duration::from_secs(1), Duration::from_secs(1)),
        50
    );
    assert_eq!(
        Timeline::new(48000, 1000, false, 0).idle_frames(
            0,
            Duration::from_secs(1),
            Duration::from_secs(1)
        ),
        0
    );
}

#[test]
fn resume_trims_overlap_and_fills_only_uncovered_time() {
    let now = Instant::now();
    let mut timeline = Timeline::new(48000, u64::MAX, true, 0);
    timeline.mark_silence();
    let packet = plan(&mut timeline, 4800, 0, 960, 90, now).unwrap();
    assert_eq!(
        (
            packet.prefix_silence,
            packet.skip,
            packet.used,
            packet.repaired_gap
        ),
        (0, 480, 480, 0)
    );
    timeline.mark_silence();
    let packet = plan(&mut timeline, 5280, 960, 480, 120, now).unwrap();
    assert_eq!(
        (packet.prefix_silence, packet.skip, packet.used),
        (480, 0, 480)
    );
}

#[test]
fn fully_overlapped_packets_preserve_silence_alignment() {
    let now = Instant::now();
    let mut timeline = Timeline::new(48000, u64::MAX, true, 0);
    timeline.mark_silence();
    assert_eq!(plan(&mut timeline, 4800, 0, 480, 80, now).unwrap().used, 0);
    let packet = plan(&mut timeline, 4800, 480, 960, 90, now).unwrap();
    assert_eq!((packet.skip, packet.used), (480, 480));
    assert!(plan(&mut timeline, 5280, 1440, 480, 89, now).is_err());
    timeline.mark_silence();
    assert!(
        timeline
            .plan(5280, 1440, 480, 10_000_000, false, Duration::ZERO, now)
            .is_err()
    );
}

#[test]
fn gap_repair_count_expires_after_sixty_seconds() {
    let now = Instant::now();
    for expire in [false, true] {
        let mut timeline = Timeline::new(48000, u64::MAX, false, 0);
        plan(&mut timeline, 0, 0, 480, 0, now).unwrap();
        for index in 1..=5 {
            let packet = plan(
                &mut timeline,
                index * 960,
                index * 960,
                480,
                index * 20,
                now,
            )
            .unwrap();
            assert_eq!(packet.repaired_gap, 480);
        }
        let packet = plan(
            &mut timeline,
            5760,
            5760,
            480,
            120,
            now + Duration::from_secs(if expire { 61 } else { 60 }),
        );
        assert_eq!(packet.is_ok(), expire);
    }
}

#[test]
fn gap_duration_budget_and_final_target_are_enforced() {
    let now = Instant::now();
    let mut timeline = Timeline::new(48000, u64::MAX, false, 0);
    plan(&mut timeline, 0, 0, 480, 0, now).unwrap();
    plan(&mut timeline, 480, 12480, 480, 260, now).unwrap();
    plan(&mut timeline, 12960, 24960, 480, 520, now).unwrap();
    assert!(plan(&mut timeline, 25440, 25920, 480, 540, now).is_err());
    let mut timeline = Timeline::new(48000, 1000, false, 0);
    plan(&mut timeline, 0, 0, 480, 0, now).unwrap();
    let packet = plan(&mut timeline, 480, 1440, 480, 30, now).unwrap();
    assert_eq!((packet.prefix_silence, packet.used), (520, 0));
}
