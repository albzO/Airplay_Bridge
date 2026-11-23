use super::*;

#[test]
fn idle_capture_stops_unseen_and_returns_only_if_it_was_paused() {
    assert_eq!(next_step(false, false, true, false), Some(Step::Pause));
    assert_eq!(next_step(false, false, false, true), None);
    assert_eq!(next_step(true, false, false, true), Some(Step::Resume));
    // 用户自己关闭的采集，恢复窗口时保持关闭。
    // Capture the user turned off stays off when the window returns.
    assert_eq!(next_step(true, false, false, false), None);
    assert_eq!(next_step(true, false, true, false), None);
}

#[test]
fn streaming_or_a_running_job_is_left_alone() {
    for seen in [true, false] {
        for (capturing, paused) in [(true, false), (false, true)] {
            assert_eq!(next_step(seen, true, capturing, paused), None);
        }
    }
}
