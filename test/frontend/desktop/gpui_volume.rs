use super::*;

#[test]
fn receiver_volume_parses_query_acknowledgment_and_rejection() {
    assert_eq!(
        VolumeReport::from_native("[PROBE] VOLUME_CURRENT source=device db=-144.000"),
        Some(VolumeReport::Level(0.))
    );
    assert_eq!(
        VolumeReport::from_native("[PROBE] VOLUME_CURRENT source=query db=-15.000"),
        Some(VolumeReport::Level(50.))
    );
    assert_eq!(
        VolumeReport::from_native("[PROBE] VOLUME_APPLIED percent=40.50 db=-17.850 http=200"),
        Some(VolumeReport::Level(40.5))
    );
    assert_eq!(
        VolumeReport::from_native("[PROBE] VOLUME_APPLIED percent=40.50 http=500"),
        Some(VolumeReport::Rejected)
    );
    assert_eq!(
        VolumeReport::from_native("[PROBE] VOLUME_CURRENT db=NaN"),
        None
    );
}

#[test]
fn unmute_restores_the_last_confirmed_nonzero_level() {
    let mut volume = Volume::default();
    volume.confirm(37.);
    assert_eq!(volume.mute_target(), 0.);
    volume.confirm(0.);
    assert_eq!(volume.mute_target(), 37.);
}
