use super::Cfg;
use crate::{DefaultKeyConfig, DefaultMapConfig, GenMap, Key, KeyConfig, MapConfig, Odd, Split};
use core::mem::size_of;
use std::vec::Vec;

#[test]
fn default_key_matches_default_config() {
    let mut map = GenMap::new();
    let key: Key = map.insert(1);
    let same: Key<DefaultKeyConfig> = key;
    assert_eq!(map[same], 1);
}

#[test]
fn maps_whose_configs_share_a_key_config_share_a_key_type() {
    struct Wrapping;

    impl MapConfig for Wrapping {
        type KeyConfig = DefaultKeyConfig;
        type Storage<S> = Vec<S>;
        const WRAP_ON_OVERFLOW: bool = true;
    }

    let mut retiring = GenMap::new();
    let mut wrapping = GenMap::<i32, Wrapping>::new_with_config();
    let a: Key = retiring.insert(1);
    let b: Key = wrapping.insert(2);
    assert_eq!(retiring[a], 1);
    assert_eq!(wrapping[b], 2);
}

#[test]
fn key_sizes_follow_the_config() {
    assert_eq!(size_of::<Key<Cfg<u8, u8>>>(), 2);
    assert_eq!(size_of::<Key<Cfg<u16, u16>>>(), 4);
    assert_eq!(size_of::<Key<Cfg<u32, u32>>>(), 8);
    assert_eq!(size_of::<Key<Cfg<u32, u16>>>(), 8);
    assert_eq!(size_of::<Key<Cfg<u64, u64>>>(), 16);
    assert_eq!(size_of::<Key>(), 8);
}

#[test]
fn option_of_key_costs_nothing_extra() {
    assert_eq!(size_of::<Option<Key<Cfg<u8, u8>>>>(), 2);
    assert_eq!(size_of::<Option<Key<Cfg<u16, u16>>>>(), 4);
    assert_eq!(size_of::<Option<Key<Cfg<u32, u32>>>>(), 8);
    assert_eq!(size_of::<Option<Key<Cfg<u64, u64>>>>(), 16);
}

#[test]
fn keys_are_ordered_by_index_then_generation() {
    // SAFETY: every generation is odd and everything fits a `u32`.
    let (a, b, c) = unsafe {
        (
            Key::<DefaultKeyConfig>::from_raw_parts(1, Odd::new(3).unwrap()),
            Key::<DefaultKeyConfig>::from_raw_parts(1, Odd::new(5).unwrap()),
            Key::<DefaultKeyConfig>::from_raw_parts(2, Odd::new(1).unwrap()),
        )
    };
    assert!(a < b);
    assert!(b < c);
    let mut sorted = [c, b, a];
    sorted.sort();
    assert_eq!(sorted, [a, b, c]);
}

#[test]
fn key_debug_prints_both_parts() {
    // SAFETY: 7 is odd and both parts fit a `u32`.
    let k = unsafe { Key::<DefaultKeyConfig>::from_raw_parts(4, Odd::new(7).unwrap()) };
    let text = std::format!("{k:?}");
    assert!(text.contains("idx: 4"));
    assert!(text.contains("generation: 7"));
}

#[test]
fn every_integer_type_works_as_a_config() {
    struct Mixed;
    impl KeyConfig for Mixed {
        type Idx = u64;
        type Gen = u8;
        type Layout = Split;
    }

    impl MapConfig for Mixed {
        type KeyConfig = Self;
        type Storage<S> = Vec<S>;
    }

    struct Wide;
    impl KeyConfig for Wide {
        type Idx = u128;
        type Gen = usize;
        type Layout = Split;
    }

    impl MapConfig for Wide {
        type KeyConfig = Self;
        type Storage<S> = Vec<S>;
    }

    let mut mixed = GenMap::<i32, Mixed>::new_with_config();
    let k = mixed.insert(1);
    assert_eq!(mixed[k], 1);

    let mut wide = GenMap::<i32, Wide>::default();
    let k = wide.insert(2);
    assert_eq!(wide[k], 2);
    assert_eq!(k.idx(), 0u128);
    assert_eq!(k.generation().get().get(), 1usize);
}

#[test]
fn map_is_send_and_sync_when_its_values_are() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<GenMap<i32>>();
    assert_send_sync::<Key>();
    assert_send_sync::<crate::Iter<'static, i32, DefaultMapConfig>>();
}

#[test]
fn an_index_that_does_not_fit_in_usize_matches_nothing() {
    struct Wide;
    impl KeyConfig for Wide {
        type Idx = u128;
        type Gen = u32;
        type Layout = Split;
    }

    impl MapConfig for Wide {
        type KeyConfig = Self;
        type Storage<S> = Vec<S>;
    }

    let mut map = GenMap::<i32, Wide>::new_with_config();
    let k = map.insert(1);
    // `too_wide` has the same low 64 bits as `k`'s index, so a conversion to
    // `usize` that truncated it would land on `k`'s slot.
    let too_wide = (1u128 << 64) | k.idx();
    // SAFETY: the generation is the one of a live key, and both parts fit
    // the `Split` layout of `Wide`.
    let bogus = unsafe { Key::<Wide>::from_raw_parts(too_wide, k.generation()) };

    assert!(map.get(bogus).is_none());
    assert!(map.get_mut(bogus).is_none());
    assert!(!map.contains_key(bogus));
    assert!(map.remove(bogus).is_none());
    assert_eq!(map.len(), 1);
    assert_eq!(map[k], 1);
}

#[test]
fn idx_and_generation_read_the_parts_of_a_key() {
    let mut map = GenMap::new();
    let a = map.insert("a");
    let b = map.insert("b");
    assert_eq!(a.idx(), 0);
    assert_eq!(b.idx(), 1);
    assert_eq!(a.generation().get().get(), 1);
    assert_eq!(b.generation().get().get(), 1);

    map.remove(a);
    let c = map.insert("c");
    assert_eq!(c.idx(), 0);
    assert_eq!(c.generation().get().get(), 3);
}

#[test]
fn a_generation_is_always_odd() {
    let mut map = GenMap::<i32, Cfg<u8, u8>>::new_with_config();
    let mut key = map.insert(0);
    for _ in 0..100 {
        assert!(key.generation().get().get() % 2 == 1);
        map.remove(key);
        key = map.insert(0);
    }
}

#[test]
fn from_raw_parts_rebuilds_a_key() {
    let mut map = GenMap::new();
    let key = map.insert(42);
    let (idx, generation) = (key.idx(), key.generation());

    let rebuilt = unsafe { Key::<DefaultKeyConfig>::from_raw_parts(idx, generation) };
    assert_eq!(rebuilt, key);
    assert_eq!(map.get(rebuilt), Some(&42));
}

#[test]
fn a_rebuilt_key_from_the_past_matches_nothing() {
    let mut map = GenMap::new();
    let old = map.insert(1);
    map.remove(old);
    let new = map.insert(2);
    assert_eq!(new.idx(), old.idx());

    let rebuilt = unsafe { Key::<DefaultKeyConfig>::from_raw_parts(old.idx(), old.generation()) };
    assert!(map.get(rebuilt).is_none());
    assert!(map.get_mut(rebuilt).is_none());
    assert!(map.remove(rebuilt).is_none());
    assert!(!map.contains_key(rebuilt));
    assert_eq!(map[new], 2);
}

#[test]
fn a_rebuilt_key_for_a_missing_slot_matches_nothing() {
    let mut map = GenMap::new();
    map.insert(1);
    let rebuilt = unsafe { Key::<DefaultKeyConfig>::from_raw_parts(99, Odd::new(1).unwrap()) };
    assert!(map.get(rebuilt).is_none());
}

#[test]
fn is_max_generation_is_true_only_at_the_last_generation() {
    let mut map = GenMap::<i32, Cfg<u8, u8>>::new_with_config();
    let mut key = map.insert(0);
    while key.generation().get().get() != u8::MAX {
        assert!(!key.is_max_generation());
        map.remove(key);
        key = map.insert(0);
    }
    assert!(key.is_max_generation());
    assert!(map.detach(key).is_none());

    // The slot retires when its last value is removed, so the next value
    // goes into a new slot, with a key that is not at the last generation.
    map.remove(key);
    let fresh = map.insert(1);
    assert_ne!(fresh.idx(), key.idx());
    assert!(!fresh.is_max_generation());
}
