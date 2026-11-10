use super::*;

#[test]
fn only_current_session_can_play_and_late_events_cannot_reopen_it() {
    let mut session = Session::default();
    session.start(5);
    assert!(!session.accept(4, StreamUpdate::Playing));
    assert!(session.accept(5, StreamUpdate::Ready));
    assert!(session.phase == Phase::Ready);
    assert!(session.accept(5, StreamUpdate::Playing));
    assert!(session.phase == Phase::Playing);
    session.stop();
    assert!(!session.accept(5, StreamUpdate::Password("fixture".into())));
    assert!(!session.accept(5, StreamUpdate::Playing));
    assert!(session.accept(
        5,
        StreamUpdate::Finished {
            error: None,
            code: None
        }
    ));
    assert!(session.phase == Phase::Stopped);
    assert!(!session.accept(5, StreamUpdate::Ready));
}

#[test]
fn rejected_password_reopens_prompt_after_cleanup() {
    let mut session = Session::default();
    session.start(8);
    assert!(session.accept(8, StreamUpdate::Password("fixture".into())));
    session.password_sent("fixture".into());
    assert!(session.prompt.is_none());
    assert!(session.accept(
        8,
        StreamUpdate::Finished {
            error: Some("Password rejected".into()),
            code: Some("PASSWORD_REJECTED".into()),
        }
    ));
    assert!(session.id.is_none());
    assert_eq!(session.prompt.as_deref(), Some("fixture"));
    session.start(9);
    assert!(session.prompt.is_none());
    assert!(!session.accept(8, StreamUpdate::Password("stale".into())));
}
