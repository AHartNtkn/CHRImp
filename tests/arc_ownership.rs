use chr::engine::Engine;
use chr::store::Store;
use std::panic::{AssertUnwindSafe, catch_unwind};
fn send<T: Send>() {}
#[test]
fn engine_and_owners_remain_send() {
    send::<Engine>();
    send::<Store<u64>>();
    send::<chr::store::Root<u64>>();
}
#[test]
fn unique_updates_cow_pins_and_bounded_release() {
    let mut s = Store::default();
    let mut r = s.empty();
    for i in 0..4096 {
        r = s.insert(r, [0, i, 0, 0], i);
    }
    assert_eq!(s.mutation_counts().1, 0);
    let old = r.clone();
    let before = s.mutation_counts();
    r = s.insert(r, [0, 0, 0, 0], 9999);
    assert!(s.mutation_counts().1 > before.1);
    assert_eq!(s.get(&old, &[0, 0, 0, 0]), Some(0));
    assert_eq!(s.get(&r, &[0, 0, 0, 0]), Some(9999));
    drop(old);
    drop(r);
    let mut ticks = 0;
    while s.release_pending() {
        let before = s.node_count();
        s.release_tick();
        assert!(before - s.node_count() <= 2);
        ticks += 1;
        assert!(ticks < 20000);
    }
    assert_eq!(s.node_count(), 0);
    assert!(ticks > 4000);
}
#[test]
fn aborted_epoch_does_not_stale_and_completed_epoch_cannot_revive() {
    let mut s = Store::default();
    let mut r = s.empty();
    for i in 0..256 {
        r = s.insert(r, [0, i, 0, 0], i);
    }
    let mut c = s.collect([r.clone()].into_iter());
    c.tick(&mut s);
    drop(c);
    assert_eq!(s.get(&r, &[0, 0, 0, 0]), Some(0));
    let mut c = s.collect(std::iter::empty());
    while !c.done() {
        c.tick(&mut s);
    }
    drop(c);
    assert!(!s.contains(&r));
    assert!(s.node_count() > 0);
    assert!(catch_unwind(AssertUnwindSafe(|| s.get(&r, &[0, 0, 0, 0]))).is_err());
    let mut c = s.collect([r.clone()].into_iter());
    assert!(catch_unwind(AssertUnwindSafe(|| c.tick(&mut s))).is_err());
    drop(c);
    drop(r);
    while !s.release_tick() {}
    assert_eq!(s.node_count(), 0);
}
#[test]
fn physical_root_may_outlive_owner_without_recursive_destruction() {
    let r = {
        let mut s = Store::default();
        let mut r = s.empty();
        for i in 0..20000 {
            r = s.insert(r, [0, i, 0, 0], i);
        }
        r
    };
    drop(r);
}

#[test]
fn cached_interval_counts_follow_unique_and_shared_paths() {
    let mut s = Store::default();
    let mut root = s.empty();
    let mut map = std::collections::BTreeMap::new();
    let mut random = 7u64;
    for i in 0..512 {
        let mut key = [0; 4];
        for x in &mut key {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            *x = random;
        }
        map.insert(key, i);
        root = s.insert(root, key, i);
    }
    let snapshot = root.clone();
    let old = map.clone();
    for (&key, _) in old.iter().step_by(3) {
        root = s.remove(root, &key);
        map.remove(&key);
    }
    for records in [&old, &map] {
        let r = if records.len() == old.len() {
            &snapshot
        } else {
            &root
        };
        assert_eq!(s.count(r, [0; 4], [u64::MAX; 4]), records.len());
        for (&key, _) in old.iter().step_by(7) {
            assert_eq!(s.count(r, [0; 4], key), records.range(..=key).count());
            assert_eq!(s.count(r, key, [u64::MAX; 4]), records.range(key..).count());
        }
    }
    let mut r = s.empty();
    for bit in 0..256 {
        let mut key = [0; 4];
        key[bit / 64] = 1 << (63 - bit % 64);
        r = s.insert(r, key, 0);
        assert_eq!(s.count(&r, [0; 4], [u64::MAX; 4]), bit + 1);
    }
    assert_eq!(s.count(&r, [0; 4], [0, 0, 0, 1]), 1);
}
