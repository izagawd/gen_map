use crate::tests::{assert_not_detached, Cfg};
use crate::{FullError, GenMap, GenMapConfig, GenSlotItem, KeepLen, MapConfig, Packed};
use std::vec::Vec;

/// `u8` keys, so a map holds at most 255 slots, and the largest generation is
/// 255.
type Byte = Cfg<u8, u8>;

/// Keys whose generation is 4 bits, so the largest one is 15, in a map that
/// wraps generations.
struct Wrap4;

impl MapConfig for Wrap4 {
    type KeyConfig = Packed<u16, 4>;
}

impl GenMapConfig for Wrap4 {
    const WRAP_ON_OVERFLOW: bool = true;
    type Len = KeepLen;
    type Storage<S: GenSlotItem> = Vec<S>;
}

/// A map with no values, whose next key gets the largest generation its key
/// config can hold.
fn before_the_largest_generation<C: GenMapConfig>() -> GenMap<i32, C>
where
    GenMap<i32, C>: Default,
{
    let mut map = GenMap::<i32, C>::default();
    while !map.vacant_entry().unwrap().key().is_max_generation() {
        let key = map.insert(0);
        map.remove(key);
    }
    map
}

#[test]
fn reserve_key_returns_the_key_the_next_insert_would_get() {
    let mut map = GenMap::<i32>::new();
    let promised = map.vacant_entry().unwrap().key();
    let key = map.reserve_key();
    assert_eq!(key, promised);
    assert!(map.get(key).is_none());
    assert!(!map.contains_key(key));
    assert_eq!(map.key_at(key.idx().get()), None);
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 1);
    // The slot is detached, so its generation is one more than the key's.
    assert_eq!(map.generation_at(key.idx().get()), Some(2));
}

#[test]
fn reserve_key_takes_the_first_free_slot() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    let b = map.insert(2);
    map.insert(3);
    map.remove(a);
    map.remove(b);

    // The slot of `b` is first on the free list, and the slot of `a` is next.
    let key = map.reserve_key();
    assert_eq!(key.idx(), b.idx());
    assert!(key.generation() > b.generation());
    assert_eq!(map.insert(4).idx(), a.idx());
    assert_eq!(map.insert(5).idx().get(), 3);
}

#[test]
fn an_insert_never_takes_a_reserved_slot() {
    let mut map = GenMap::new();
    let key = map.reserve_key();
    let others: Vec<_> = (0..10).map(|i| map.insert(i)).collect();
    for other in others {
        assert_ne!(other.idx(), key.idx());
    }
    map.attach(key, 10).unwrap();
    assert_eq!(map[key], 10);
    assert_eq!(map.len(), 11);
}

#[test]
fn attach_puts_a_value_under_a_reserved_key_once() {
    let mut map = GenMap::new();
    let key = map.reserve_key();
    map.attach(key, 1).unwrap();
    assert_eq!(map.get(key), Some(&1));
    assert_eq!(map.key_at(key.idx().get()), Some(key));
    assert_eq!(map.len(), 1);
    assert_not_detached(&mut map, key, 2);

    // From here on, the key works like a key that `insert` returned.
    assert_eq!(map.remove(key), Some(1));
    assert!(map.get(key).is_none());
}

#[test]
fn lookups_and_removals_ignore_a_reserved_key() {
    let mut map = GenMap::<i32>::new();
    let key = map.reserve_key();
    assert!(map.get_mut(key).is_none());
    assert_eq!(map.remove(key), None);
    assert_eq!(map.retire(key), None);
    assert_eq!(map.detach(key), None);
    assert_eq!(map.iter().count(), 0);

    // None of the calls above freed the slot, so it is still reserved.
    map.attach(key, 1).unwrap();
    assert_eq!(map[key], 1);
}

#[test]
fn release_frees_a_reserved_slot() {
    let mut map = GenMap::new();
    let key = map.reserve_key();
    assert!(map.release(key));
    assert!(!map.release(key));
    assert_eq!(map.attach(key, 1), Err(1));
    assert_eq!(map.len(), 0);

    // The freed slot is first on the free list, so the next insert takes it
    // under a newer generation.
    let other = map.insert(2);
    assert_eq!(other.idx(), key.idx());
    assert!(other.generation() > key.generation());
    assert!(map.get(key).is_none());
}

#[test]
fn attach_and_release_fail_for_a_key_reserved_before_a_reset() {
    let mut map = GenMap::new();
    let key = map.reserve_key();
    map.reset();
    assert!(!map.release(key));
    assert_not_detached(&mut map, key, 1);
    assert_eq!(map.slots_len(), 0);
}

#[test]
fn clear_retain_and_clone_leave_a_reserved_slot_reserved() {
    let mut map = GenMap::new();
    let a = map.reserve_key();
    let b = map.insert(1);
    map.retain(|_, _| true);
    map.clear();
    assert!(map.get(b).is_none());

    let mut clone = map.clone();
    clone.attach(a, 2).unwrap();
    assert_eq!(clone[a], 2);
    map.attach(a, 3).unwrap();
    assert_eq!(map[a], 3);
}

#[test]
#[should_panic(expected = "cannot address more than 255 slots")]
fn reserve_key_panics_when_the_index_runs_out() {
    let mut map = GenMap::<i32, Byte>::new_with_config();
    for _ in 0..256 {
        map.reserve_key();
    }
}

#[test]
fn try_reserve_key_reports_index_exhausted_and_adds_no_slot() {
    let mut map = GenMap::<i32, Byte>::new_with_config();
    let keys: Vec<_> = (0..255).map(|_| map.try_reserve_key().unwrap()).collect();
    assert_eq!(map.try_reserve_key(), Err(FullError::IndexExhausted));
    assert_eq!(map.slots_len(), 255);
    assert_eq!(map.len(), 0);

    // Releasing a key puts its slot on the free list, so the next key gets it.
    assert!(map.release(keys[7]));
    assert_eq!(map.try_reserve_key().unwrap().idx(), keys[7].idx());
}

#[test]
fn releasing_a_key_reserved_at_the_largest_generation_retires_its_slot() {
    let mut map = before_the_largest_generation::<Byte>();
    let key = map.reserve_key();
    assert_eq!(key.generation().get().get(), u8::MAX);
    // The generation of the detached slot wraps to zero, so the key finds
    // nothing.
    assert_eq!(map.generation_at(key.idx().get()), Some(0));
    assert!(map.get(key).is_none());

    assert!(map.release(key));
    assert_eq!(map.generation_at(key.idx().get()), Some(0));
    assert_eq!(map.attach(key, 1), Err(1));
    // The retired slot is skipped, so the next value gets a new slot.
    assert_ne!(map.insert(1).idx(), key.idx());
}

#[test]
fn releasing_a_key_reserved_at_the_largest_generation_wraps_its_slot() {
    let mut map = before_the_largest_generation::<Wrap4>();
    let key = map.reserve_key();
    assert_eq!(key.generation().get().get(), 15);
    // The generation goes to 16, one past the largest a key can hold. It is
    // even, so no key matches the slot.
    assert_eq!(map.generation_at(key.idx().get()), Some(16));

    assert!(map.release(key));
    assert_eq!(map.generation_at(key.idx().get()), Some(0));
    let next = map.insert(1);
    assert_eq!(next.idx(), key.idx());
    assert_eq!(next.generation().get().get(), 1);
}

#[test]
fn a_key_reserved_at_the_largest_generation_takes_a_value() {
    let mut map = before_the_largest_generation::<Byte>();
    let key = map.reserve_key();
    map.attach(key, 1).unwrap();
    assert_eq!(map[key], 1);
    assert_eq!(map.generation_at(key.idx().get()), Some(u8::MAX));

    // Removing the value retires the slot, as it does for a key that `insert`
    // returned at this generation.
    assert_eq!(map.remove(key), Some(1));
    assert_ne!(map.insert(2).idx(), key.idx());
}
