use super::*;
#[test]
fn two_hour_clock_mismatch_and_jitter_stay_bounded() {
    for hardware in [-500.0, -100.0, 0.0, 100.0, 500.0] {
        let mut controller = Controller::new();
        let mut water = 136.0;
        for second in 0..7200 {
            let jitter = 4.0 * (second as f64 * 1.7).sin();
            let ppm = controller.update(water + jitter, 1.0);
            water += (hardware + ppm * (1.0 + hardware / 1e6)) * 0.001;
            assert!(
                (90.0..180.0).contains(&water),
                "hardware={hardware} water={water}"
            );
        }
        assert!((controller.correction_ppm + hardware).abs() < 50.0);
    }
}
#[test]
fn rate_step_slew_limit_and_limit_recovery() {
    let mut controller = Controller::new();
    let mut water = 136.0;
    let mut previous = 0.0;
    for second in 0..10800 {
        let hardware = if second < 3600 { 300.0 } else { -300.0 };
        let ppm = controller.update(water, 1.0);
        assert!((ppm - previous).abs() <= 20.000001);
        assert!(ppm.abs() <= 800.0);
        water += (hardware + ppm) * 0.001;
        assert!((90.0..180.0).contains(&water));
        previous = ppm;
    }
    assert!((controller.correction_ppm - 300.0).abs() < 1.0);
    for _ in 0..1000 {
        controller.update(1000.0, 1.0);
    }
    assert_eq!(controller.correction_ppm, -800.0);
    for _ in 0..1000 {
        controller.update(controller.target_ms - 10.0, 1.0);
    }
    assert!(controller.correction_ppm > 0.0);
}
