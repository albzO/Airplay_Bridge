use super::*;
#[test]
fn qpc_conversion_uses_wide_intermediates_and_rejects_invalid_clock_values() {
    assert_eq!(ticks_to_100ns(3, 2).unwrap(), 15_000_000);
    assert_eq!(
        ticks_to_100ns(i64::MAX, 10_000_000).unwrap(),
        i64::MAX as u128
    );
    assert!(ticks_to_100ns(-1, 1).is_err());
    assert!(ticks_to_100ns(1, 0).is_err());
    assert!(ticks_to_100ns(1, -1).is_err());
    assert_eq!(origin(false).unwrap(), 0);
}
