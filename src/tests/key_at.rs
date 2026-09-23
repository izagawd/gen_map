use super::Cfg;
use crate::{Config, GenMap};
use std::vec::Vec;

#[test]
fn key_at_finds_the_key_of_an_occupied_slot() {
    let mut map = GenMap::new();
    let a = map.insert("a");
    let b = map.insert("b");
    assert_eq!(map.key_at(0), Some(a));
    assert_eq!(map.key_at(1), Some(b));
    assert_eq!(map[map.key_at(1).unwrap()], "b");
}

#[test]
fn key_at_is_none_for_a_missing_slot() {
    let mut map = GenMap::new();
    assert_eq!(map.key_at(0), None);
    map.insert(1);
    assert_eq!(map.key_at(1), None);
    assert_eq!(map.key_at(u32::MAX), None);
}

#[test]
fn key_at_is_none_for_a_vacant_slot() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    map.remove(key);
    assert_eq!(map.key_at(key.idx()), None);
}

#[test]
fn key_at_reports_the_current_key_of_a_reused_slot() {
    let mut map = GenMap::new();
    let old = map.insert(1);
    map.remove(old);
    let new = map.insert(2);
    assert_eq!(new.idx(), old.idx());
    assert_eq!(map.key_at(old.idx()), Some(new));
    assert_ne!(map.key_at(old.idx()), Some(old));
}

#[test]
fn key_at_is_none_for_a_retired_slot() {
    let mut map = GenMap::<i32, Cfg<u8, u8>>::new_with_config();
    let mut key = map.insert(0);
    while map.slots_len() == 1 {
        map.remove(key);
        key = map.insert(0);
    }
    // The first slot retired and the value went into a second one.
    assert_eq!(key.idx(), 1);
    assert_eq!(map.key_at(0), None);
    assert_eq!(map.key_at(1), Some(key));
}

#[test]
fn key_at_is_none_for_a_detached_slot() {
    let mut map = GenMap::new();
    let key = map.insert(1);
    let value = map.detach(key).unwrap();
    assert_eq!(map.key_at(key.idx()), None);
    map.reattach(key, value);
    assert_eq!(map.key_at(key.idx()), Some(key));
}

#[test]
fn key_at_with_an_index_that_does_not_fit_in_usize_is_none() {
    struct Wide;
    impl Config for Wide {
        type Idx = u128;
        type Gen = u32;
        type Storage<S> = Vec<S>;
    }

    let mut map = GenMap::<i32, Wide>::new_with_config();
    let key = map.insert(1);
    assert_eq!(map.key_at(key.idx()), Some(key));
    assert_eq!(map.key_at(u128::MAX), None);
}

#[test]
fn key_at_agrees_with_iteration() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..50).map(|i| map.insert(i)).collect();
    for key in keys.iter().step_by(3) {
        map.remove(*key);
    }
    for (key, _) in &map {
        assert_eq!(map.key_at(key.idx()), Some(key));
    }
    for key in keys.iter().step_by(3) {
        assert_eq!(map.key_at(key.idx()), None);
    }
}

#[test]
fn key_at_unchecked_agrees_with_key_at() {
    let mut map = GenMap::new();
    let keys: Vec<_> = (0..20).map(|i| map.insert(i)).collect();
    map.remove(keys[4]);
    let reused = map.insert(100);

    for (key, _) in &map {
        let unchecked = unsafe { map.key_at_unchecked(key.idx()) };
        assert_eq!(map.key_at(key.idx()), Some(unchecked));
        assert_eq!(unchecked, key);
    }
    assert_eq!(unsafe { map.key_at_unchecked(4) }, reused);
}

#[test]
fn key_at_then_get_is_a_valid_lookup() {
    let mut map = GenMap::new();
    let key = map.insert(7);
    let found = map.key_at(key.idx()).unwrap();
    assert_eq!(map.get(found), Some(&7));
    assert_eq!(unsafe { *map.get_unchecked(found) }, 7);
    *map.get_mut(found).unwrap() = 8;
    assert_eq!(map[key], 8);
}
