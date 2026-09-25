use crate::GenMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[test]
fn reset_empties_the_map_and_drops_every_slot() {
    let mut map = GenMap::new();
    for i in 0..32 {
        map.insert(i);
    }
    assert_eq!(map.len(), 32);

    map.reset();

    assert_eq!(map.len(), 0);
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 0);
}

#[test]
fn reset_keeps_capacity() {
    let mut map = GenMap::with_capacity(64);
    for i in 0..64 {
        map.insert(i);
    }
    let capacity = map.capacity();

    map.reset();

    assert_eq!(map.capacity(), capacity);
}

#[test]
fn reset_starts_keys_over() {
    let mut map = GenMap::new();
    let k = map.insert(1);
    map.remove(k);
    let k2 = map.insert(2);
    assert_eq!(k2.generation().get().get(), 3);

    map.reset();
    assert!(map.get(k2).is_none());

    let k3 = map.insert(999);
    assert_eq!(k3.idx(), 0);
    assert_eq!(k3.generation().get().get(), 1);
    assert_eq!(k3, k);
    // The key handed out before the reset now finds the new value, which is
    // the hazard the docs warn about.
    assert_eq!(map.get(k), Some(&999));
}

#[test]
fn clear_keeps_old_keys_invalid_unlike_reset() {
    let mut map = GenMap::new();
    let k = map.insert(1);
    map.clear();
    let k2 = map.insert(999);
    assert_ne!(k, k2);
    assert!(map.get(k).is_none());
}

#[test]
fn reset_on_empty_map_is_a_noop() {
    let mut map = GenMap::new();
    map.reset();
    assert_eq!(map.len(), 0);
    let k = map.insert(7);
    assert_eq!(map[k], 7);
}

#[test]
fn reset_drops_values() {
    let tracker = super::DropTracker::new();
    let mut map = GenMap::new();
    for _ in 0..4 {
        map.insert(tracker.make_item());
    }
    tracker.assert_none_dropped();
    map.reset();
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn reset_stays_consistent_when_a_drop_panics() {
    struct Bomb(bool);
    impl Drop for Bomb {
        fn drop(&mut self) {
            if self.0 && !std::thread::panicking() {
                panic!("boom");
            }
        }
    }

    let mut map = GenMap::new();
    map.insert(Bomb(true));
    map.insert(Bomb(false));
    let vacant = map.insert(Bomb(false));
    // Leaves a slot on the free list, which the reset must forget.
    assert!(map.remove(vacant).is_some());

    assert!(catch_unwind(AssertUnwindSafe(|| map.reset())).is_err());

    // The map must look empty, not half reset, so the free list cannot
    // point at slots that no longer exist.
    assert!(map.is_empty());
    assert_eq!(map.slots_len(), 0);
    let k = map.insert(Bomb(false));
    assert!(map.contains_key(k));
    assert_eq!(map.slots_len(), 1);
}
