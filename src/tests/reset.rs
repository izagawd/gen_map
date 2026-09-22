use crate::GenMap;

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
    assert_eq!(k2.generation(), 3);

    map.reset();
    assert!(map.get(k2).is_none());

    let k3 = map.insert(999);
    assert_eq!(k3.index(), 0);
    assert_eq!(k3.generation(), 1);
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
