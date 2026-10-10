use super::*;
#[test]
fn observed_progress_and_successful_delivery_keep_distinct_counter_semantics() {
    let mutex = Mutex::new(CaptureProgress::default());
    let progress = Progress(Some(&mutex));
    let mut counters = Counters::default();
    let packet = PacketInfo {
        index: 0,
        device_position: 100,
        qpc: 200,
        available: 4,
        flags: (AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 | AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0)
            as u32,
    };
    let metrics = PacketMetrics::measure(&[0.25, -0.5, 0.0, 0.0], false);
    progress.fallback();
    progress.gap(20);
    progress.silence(10);
    let p = progress.packet(packet, 2, &metrics).unwrap();
    counters.observe(packet, &metrics);
    assert_eq!(
        (p.packets, p.discontinuities, p.timestamp_errors),
        (1, 1, 1)
    );
    assert_eq!(
        (
            p.event_timeout_packets,
            p.repaired_gaps,
            p.repaired_gap_frames
        ),
        (1, 1, 20)
    );
    assert_eq!(
        (p.synthesized_silent_frames, p.skipped_overlap_frames),
        (10, 2)
    );
    assert_eq!(
        (
            p.device_position,
            p.packet_qpc_100ns,
            p.packet_frames,
            p.packet_flags
        ),
        (100, 200, 4, packet.flags)
    );
    // sink 失败时不提交交付计数；已经观察到的错误与信号仍保留。
    // Failed sinks do not commit delivery counters; observed faults and signal remain.
    assert_eq!(
        (
            counters.frames,
            counters.packets,
            counters.discontinuities,
            counters.timestamp_errors
        ),
        (0, 0, 0, 1)
    );
    assert_eq!(counters.signal_frames, 1);
    counters.delivered_silence(10);
    counters.delivered_packet(2);
    counters.observe(PacketInfo { index: 1, ..packet }, &metrics);
    assert_eq!(
        (
            counters.frames,
            counters.packets,
            counters.silent_frames,
            counters.discontinuities
        ),
        (12, 1, 10, 1)
    );
    assert_eq!(counters.channel_peaks, [0.25, 0.5]);
}
#[test]
fn optional_progress_never_requires_a_shared_snapshot() {
    let progress = Progress(None);
    progress.fallback();
    progress.gap(5);
    progress.silence(5);
    assert!(
        progress
            .packet(
                PacketInfo {
                    index: 0,
                    device_position: 0,
                    qpc: 0,
                    available: 0,
                    flags: 0
                },
                0,
                &PacketMetrics::measure(&[], false)
            )
            .is_none()
    );
}
