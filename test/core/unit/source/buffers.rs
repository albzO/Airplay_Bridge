use super::*;

#[test]
fn reuse_preserves_float_bits_frame_order_lengths_and_owned_copy() {
    let (mut pool, returned) = Pool::new();
    let original = [0.25, -0.5, -0.0, f32::from_bits(0x7fc01234)];
    let mut input = original;
    let first = pool.copy(&input);
    input.fill(9.0);
    assert_eq!(
        first.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        original.map(f32::to_bits)
    );
    let pointer = first.as_ptr();
    recycle(&returned, first);
    let small = pool.copy(&[-1.0, 0.75]);
    assert_eq!(small, [-1.0, 0.75]);
    assert_eq!(small.as_ptr(), pointer);
    recycle(&returned, small);
    let large = pool.copy(&[0.125; 100]);
    assert_eq!(large, [0.125; 100]);
    assert_eq!(
        pool.stats.snapshot(),
        json!({"created":1,"reused":2,"growths":1})
    );
}

#[test]
fn count_capacity_limits_disconnect_and_new_subscription_bound_retention() {
    let (mut pool, returned) = Pool::new();
    recycle(&returned, vec![1.; MAX_BUFFER_SAMPLES + 1]);
    assert!(pool.returned.try_recv().is_err());
    for _ in 0..BUFFER_COUNT + 3 {
        recycle(&returned, Vec::with_capacity(MAX_BUFFER_SAMPLES));
    }
    let buffers: Vec<_> = pool.returned.try_iter().collect();
    assert_eq!(buffers.len(), BUFFER_COUNT);
    assert!(
        buffers
            .iter()
            .all(|buffer| buffer.capacity() <= MAX_BUFFER_SAMPLES)
    );
    let copy = pool.copy(&[0.25, -0.5]);
    recycle(&returned, copy);
    drop(pool);
    recycle(&returned, vec![0.; 2]);
    let (mut next, _) = Pool::new();
    assert_eq!(next.copy(&[0.75, -1.0]), [0.75, -1.0]);
    assert_eq!(
        next.stats.snapshot(),
        json!({"created":1,"reused":0,"growths":0})
    );
}
