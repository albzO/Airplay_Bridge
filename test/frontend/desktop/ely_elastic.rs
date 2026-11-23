use super::*;

/// 内容 1000、可视区 600，可滚动 400。
/// Content 1000, viewport 600: 400 of scrolling.
fn scroller() -> Elastic {
    let elastic = Elastic::default();
    elastic.content.set(1000.);
    elastic.viewport.set(600.);
    elastic
}

/// 以 6 ms 一帧推进，返回每帧位移。
/// Advances in 6 ms frames and returns each frame's shift.
fn run(elastic: &mut Elastic, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|_| {
            elastic.ticked -= Duration::from_millis(6);
            elastic.moved -= Duration::from_millis(6);
            f32::from(elastic.frame().0)
        })
        .collect()
}

fn largest_step(shown: &[f32]) -> f32 {
    shown
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0., f32::max)
}

#[test]
fn a_notch_in_the_middle_glides_instead_of_jumping() {
    let mut elastic = scroller();
    elastic.wheel(-120.);
    let shown = run(&mut elastic, 120);
    assert!(largest_step(&shown) < 12., "step {}", largest_step(&shown));
    assert_eq!(*shown.last().unwrap(), -120.);
    assert!(!elastic.frame().1);
}

#[test]
fn scrolling_stops_at_the_limit_before_it_stretches() {
    let mut elastic = scroller();
    elastic.wheel(-120.);
    elastic.wheel(-120.);
    elastic.wheel(-120.);
    // 这一下越过底端：停在 400，不拉伸。
    // This one passes the bottom: it stops at 400 without stretching.
    elastic.wheel(-120.);
    assert_eq!(elastic.scroll.target, 400.);
    assert_eq!(elastic.pull.target, 0.);
    // 已在底端继续滚才拉伸，方向向上。
    // Only scrolling on from the bottom stretches, upwards.
    elastic.wheel(-120.);
    assert!(elastic.pull.target < 0.);
    let shown = run(&mut elastic, 300);
    assert_eq!(*shown.last().unwrap(), -400.);
}

#[test]
fn the_pull_at_the_top_is_bounded_eases_out_and_does_not_overshoot() {
    let mut elastic = scroller();
    for _ in 0..30 {
        elastic.wheel(120.);
    }
    assert!(elastic.pull.target <= REACH && elastic.pull.target > 60.);
    let shown = run(&mut elastic, 300);
    assert!(largest_step(&shown) < 8.);
    assert!(shown.iter().all(|&y| y > -0.5));
    assert_eq!(*shown.last().unwrap(), 0.);
}

#[test]
fn shorter_content_pulls_the_target_back_into_range() {
    let mut elastic = scroller();
    for _ in 0..4 {
        elastic.wheel(-120.);
    }
    elastic.content.set(700.);
    let shown = run(&mut elastic, 300);
    assert_eq!(*shown.last().unwrap(), -100.);
}
