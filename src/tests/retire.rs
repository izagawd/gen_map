//! Retiring a slot by hand with `GenMap::retire`. The randomized model test
//! covers it too.

use super::DropTracker;
use crate::{Config, GenMap, InsertError, Packed};
use std::vec::Vec;

/// Sixteen slots whose generations wrap after eight values, so every slot is
/// used again unless it is retired by hand.
struct Wrapping;

impl Config for Wrapping {
    type Idx = u8;
    type Gen = u8;
    type Layout = Packed<u8, 4>;
    type Storage<S> = Vec<S>;
    const WRAP_ON_OVERFLOW: bool = true;
}

#[test]
fn retire_takes_the_value_out_and_retires_the_slot() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);

    assert_eq!(map.retire(a), Some(1));
    assert_eq!(map.len(), 1);
    assert!(map.get(a).is_none());
    assert!(!map.contains_key(a));
    assert!(map.key_at(a.idx()).is_none());
    assert_eq!(map.generation_at(a.idx()), Some(0));
    assert_eq!(map[b], 2);

    // The retired slot is skipped, so the next value goes into a new slot.
    let c = map.insert(3);
    assert_eq!(c.idx(), 2);
    assert_eq!(map.slots_len(), 3);
}

#[test]
fn retire_with_an_invalid_key_is_none_and_changes_nothing() {
    let mut map = GenMap::new();
    let stale = map.insert(1);
    map.remove(stale);
    let live = map.insert(2);
    let detached = map.insert(3);
    assert_eq!(map.detach(detached), Some(3));

    let mut other = GenMap::new();
    let missing = (0..10).map(|i| other.insert(i)).last().unwrap();

    assert!(map.retire(stale).is_none());
    assert!(map.retire(detached).is_none());
    assert!(map.retire(missing).is_none());
    assert_eq!(map.len(), 1);
    assert_eq!(map[live], 2);

    // The detached slot was left alone, so its key still works.
    map.reattach(detached, 30);
    assert_eq!(map[detached], 30);

    assert_eq!(map.retire(live), Some(2));
    assert!(map.retire(live).is_none());
}

/// Code on top of a map that wraps can retire a slot instead of letting it
/// wrap, while the config keeps wrapping every other slot.
#[test]
fn retire_keeps_a_chosen_slot_from_wrapping() {
    let mut map = GenMap::<u32, Wrapping>::new_with_config();

    // Left to the config, slot 0 wraps and hands out its first key again.
    let first = map.insert(0);
    let mut key = first;
    while !key.is_max_generation() {
        map.remove(key);
        key = map.insert(0);
        assert_eq!(key.idx(), 0);
    }
    assert_eq!(key.generation(), 15);
    map.remove(key);
    assert_eq!(map.insert(0), first);

    // This time the slot is retired at its last generation instead.
    let mut key = first;
    while !key.is_max_generation() {
        map.remove(key);
        key = map.insert(0);
    }
    assert_eq!(map.retire(key), Some(0));
    let fresh = map.insert(1);
    assert_eq!(fresh.idx(), 1);
    assert!(map.get(first).is_none());
    assert_eq!(map.generation_at(0), Some(0));
}

#[test]
fn retired_slots_count_toward_a_full_map() {
    let mut map = GenMap::<u32, Wrapping>::new_with_config();
    for i in 0..16 {
        let key = map.insert(i);
        assert_eq!(map.retire(key), Some(i));
    }
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 16);
    assert!(matches!(
        map.try_insert(16),
        Err(InsertError::IndexExhausted(16))
    ));
}

#[test]
fn a_retired_slot_stays_retired_until_reset() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    map.insert(2);
    assert_eq!(map.retire(a), Some(1));

    map.retain(|_, _| true);
    drop(map.drain());
    map.clear();
    let mut copy = map.clone();
    for m in [&mut map, &mut copy] {
        assert_eq!(m.generation_at(a.idx()), Some(0));
        let key = m.insert(3);
        assert_ne!(key.idx(), a.idx());
    }

    map.reset();
    let b = map.insert(4);
    assert_eq!(b.idx(), a.idx());
    assert_eq!(map[b], 4);
}

#[test]
#[should_panic(expected = "not detached")]
fn reattach_panics_for_a_retired_key() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.retire(key);
    map.reattach(key, 2);
}

#[test]
fn retire_hands_the_value_back_without_dropping_it() {
    let tracker = DropTracker::new();
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..2).map(|_| map.insert(tracker.make_item())).collect();

    let item = map.retire(keys[0]).unwrap();
    tracker.assert_none_dropped();
    drop(item);
    drop(map);
    tracker.assert_all_dropped_exactly_once(2);
}
