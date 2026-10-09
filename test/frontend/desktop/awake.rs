use super::*;
#[test]
fn system_request_can_be_enabled_and_released_without_reference_leaks() {
    let mut request = Awake::default();
    request.set(true).unwrap();
    request.set(true).unwrap();
    assert!(request.active());
    request.set(false).unwrap();
    request.set(false).unwrap();
    assert!(!request.active());
}
