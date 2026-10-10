use super::*;

#[test]
fn user_stop_preserves_real_failures_and_reports_unconfirmed_cleanup() {
    for cause in [
        "[BACKEND_STDERR_FAILED] fixture read failure",
        "[STOP_TIMEOUT] fixture timeout",
        "[PASSWORD_REJECTED] fixture protocol failure",
    ] {
        assert_eq!(
            finished_error(Err(cause.into()), true, true).as_deref(),
            Some(cause)
        );
        let error = finished_error(Err(cause.into()), true, false).unwrap();
        assert!(error.starts_with(cause));
        assert!(error.contains("重启应用"));
    }
    assert!(finished_error(Ok(()), true, true).is_none());
    let cancelled = homepod_test::failure::describe("已取消建立流", "STREAM_FAILED");
    assert!(finished_error(Err(cancelled.clone()), true, true).is_none());
    assert!(finished_error(Err(cancelled), false, true).is_some());
    assert!(
        finished_error(Ok(()), true, false)
            .unwrap()
            .contains("STOP_TIMEOUT")
    );
}
