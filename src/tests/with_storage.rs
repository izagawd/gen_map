//! These tests cover `with_storage` and `with_storages`. They check that a
//! map built from empty storages keeps them and works like any other map, and
//! that the error hands back storages that hold items without changing or
//! dropping those items.

use super::{Cfg, DropTracker};
use crate::{
    DenseGenMap, DenseSecondaryMap, GenMap, PairStorage, PairVec, SecondaryMap, WithStorageError,
    WithStoragesError,
};
use std::format;
use std::vec::Vec;

/// Maps with this config keep their slots in a `Vec`, and the dense maps keep
/// their keys and values in a `PairVec`.
type Config = Cfg<u32, u32>;

#[test]
fn a_gen_map_built_from_an_empty_storage_keeps_it_and_works() {
    let storage = Vec::with_capacity(8);
    let capacity = storage.capacity();
    let mut map = GenMap::<i32, Config>::with_storage(storage).unwrap();
    assert_eq!(map.capacity(), capacity);
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 0);

    let a = map.insert(1);
    let b = map.insert(2);
    assert_eq!(map.remove(a), Some(1));
    let c = map.insert(3);
    assert_eq!(c.idx(), a.idx());
    assert!(map.get(a).is_none());
    assert_eq!(map[b], 2);
    assert_eq!(map[c], 3);
    assert_eq!(map.len(), 2);
}

#[test]
fn a_gen_map_refuses_a_storage_that_holds_slots_and_hands_it_back() {
    let mut map = GenMap::<i32, Config>::new_with_config();
    let a = map.insert(1);
    map.insert(2);
    map.remove(a);
    let slots = map.into_raw_parts().slots;

    let error = GenMap::<i32, Config>::with_storage(slots).unwrap_err();
    assert!(matches!(error, WithStorageError::NotEmpty(_)));
    assert_eq!(format!("{error}"), "the storage is not empty");
    assert_eq!(format!("{error:?}"), "NotEmpty(..)");

    // The storage still holds both slots, including the free one.
    let slots = error.into_storage();
    assert_eq!(slots.len(), 2);
    assert!(slots[a.idx() as usize].get_odd(a.generation()).is_none());
}

#[test]
fn a_secondary_map_built_from_an_empty_storage_keeps_it_and_works() {
    let mut keys = GenMap::<(), Config>::new_with_config();
    let a = keys.insert(());
    let b = keys.insert(());

    let storage = Vec::with_capacity(8);
    let capacity = storage.capacity();
    let mut map = SecondaryMap::<i32, Config>::with_storage(storage).unwrap();
    assert_eq!(map.capacity(), capacity);
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 0);

    map.insert(b, 2).unwrap();
    assert!(map.get(a).is_none());
    assert_eq!(map[b], 2);
    assert_eq!(map.len(), 1);
}

#[test]
fn a_secondary_map_refuses_a_storage_that_holds_slots_and_hands_it_back() {
    let mut keys = GenMap::<(), Config>::new_with_config();
    let key = keys.insert(());
    let mut map = SecondaryMap::<i32, Config>::new_with_config();
    map.insert(key, 1).unwrap();
    let slots = map.into_raw_parts().slots;

    let error = SecondaryMap::<i32, Config>::with_storage(slots).unwrap_err();
    assert!(matches!(error, WithStorageError::NotEmpty(_)));
    let slots = error.into_storage();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].get_odd(key.generation()), Some(&1));
}

#[test]
fn a_dense_gen_map_built_from_empty_storages_keeps_them_and_works() {
    let slots = Vec::with_capacity(8);
    let pairs = PairVec::with_capacity(16);
    let capacity = slots.capacity().min(pairs.capacity());
    let mut map = DenseGenMap::<i32, Config>::with_storages(slots, pairs).unwrap();
    assert_eq!(map.capacity(), capacity);
    assert_eq!(map.len(), 0);

    let a = map.insert(1);
    let b = map.insert(2);
    assert_eq!(map.remove(a), Some(1));
    let c = map.insert(3);
    assert!(map.get(a).is_none());
    assert_eq!(map[b], 2);
    assert_eq!(map[c], 3);
    assert_eq!(map.values().copied().collect::<Vec<_>>(), [2, 3]);
}

#[test]
fn a_dense_gen_map_says_which_storage_holds_items_and_hands_both_back() {
    let mut map = DenseGenMap::<i32, Config>::new_with_config();
    map.insert(1);
    let parts = map.into_raw_parts();

    let error = DenseGenMap::<i32, Config>::with_storages(parts.slots, PairVec::new()).unwrap_err();
    assert!(matches!(error, WithStoragesError::SlotsNotEmpty(..)));
    assert_eq!(format!("{error}"), "the slot storage is not empty");
    assert_eq!(format!("{error:?}"), "SlotsNotEmpty(..)");
    let (slots, empty_pairs) = error.into_storages();
    assert_eq!(slots.len(), 1);
    assert!(empty_pairs.is_empty());

    let error = DenseGenMap::<i32, Config>::with_storages(Vec::new(), parts.pairs).unwrap_err();
    assert!(matches!(error, WithStoragesError::PairsNotEmpty(..)));
    assert_eq!(format!("{error}"), "the pair storage is not empty");
    assert_eq!(format!("{error:?}"), "PairsNotEmpty(..)");
    let (empty_slots, pairs) = error.into_storages();
    assert!(empty_slots.is_empty());
    assert_eq!(pairs.second_slice(), [1]);

    // `with_storages` checks the slot storage first, so it returns
    // `SlotsNotEmpty` when both storages hold items.
    let error = DenseGenMap::<i32, Config>::with_storages(slots, pairs).unwrap_err();
    assert!(matches!(error, WithStoragesError::SlotsNotEmpty(..)));
    let (slots, pairs) = error.into_storages();
    assert_eq!(slots.len(), 1);
    assert_eq!(pairs.second_slice(), [1]);
}

#[test]
fn a_dense_secondary_map_built_from_empty_storages_keeps_them_and_works() {
    let mut keys = GenMap::<(), Config>::new_with_config();
    let a = keys.insert(());
    let b = keys.insert(());

    let slots = Vec::with_capacity(8);
    let pairs = PairVec::with_capacity(16);
    let capacity = slots.capacity().min(pairs.capacity());
    let mut map = DenseSecondaryMap::<i32, Config>::with_storages(slots, pairs).unwrap();
    assert_eq!(map.capacity(), capacity);
    assert_eq!(map.len(), 0);

    map.insert(b, 2).unwrap();
    assert!(map.get(a).is_none());
    assert_eq!(map[b], 2);
    assert_eq!(map.len(), 1);
}

#[test]
fn a_dense_secondary_map_says_which_storage_holds_items_and_hands_both_back() {
    let mut keys = GenMap::<(), Config>::new_with_config();
    let key = keys.insert(());
    let mut map = DenseSecondaryMap::<i32, Config>::new_with_config();
    map.insert(key, 1).unwrap();
    let parts = map.into_raw_parts();

    let error =
        DenseSecondaryMap::<i32, Config>::with_storages(parts.slots, PairVec::new()).unwrap_err();
    assert!(matches!(error, WithStoragesError::SlotsNotEmpty(..)));
    let (slots, empty_pairs) = error.into_storages();
    assert_eq!(slots.len(), 1);
    assert!(empty_pairs.is_empty());

    let error =
        DenseSecondaryMap::<i32, Config>::with_storages(Vec::new(), parts.pairs).unwrap_err();
    assert!(matches!(error, WithStoragesError::PairsNotEmpty(..)));
    let (empty_slots, pairs) = error.into_storages();
    assert!(empty_slots.is_empty());
    assert_eq!(pairs.second_slice(), [1]);

    // `with_storages` checks the slot storage first, so it returns
    // `SlotsNotEmpty` when both storages hold items.
    let error = DenseSecondaryMap::<i32, Config>::with_storages(slots, pairs).unwrap_err();
    assert!(matches!(error, WithStoragesError::SlotsNotEmpty(..)));
    let (slots, pairs) = error.into_storages();
    assert_eq!(slots.len(), 1);
    assert_eq!(pairs.first_slice(), [key]);
    assert_eq!(pairs.second_slice(), [1]);
}

#[test]
fn a_refused_storage_drops_each_value_once_when_it_is_dropped() {
    let tracker = DropTracker::new();

    let mut map = GenMap::<_, Config>::new_with_config();
    map.insert(tracker.make_item());
    map.insert(tracker.make_item());
    let slots = map.into_raw_parts().slots;
    let Err(error) = GenMap::<_, Config>::with_storage(slots) else {
        panic!("the map accepted a storage that holds slots");
    };
    tracker.assert_none_dropped();
    drop(error);
    tracker.assert_all_dropped_exactly_once(2);
}

#[test]
fn refused_dense_storages_drop_each_value_once_when_they_are_dropped() {
    let tracker = DropTracker::new();

    let mut map = DenseGenMap::<_, Config>::new_with_config();
    map.insert(tracker.make_item());
    map.insert(tracker.make_item());
    let parts = map.into_raw_parts();
    let Err(error) = DenseGenMap::<_, Config>::with_storages(parts.slots, parts.pairs) else {
        panic!("the map accepted storages that hold items");
    };
    tracker.assert_none_dropped();
    drop(error);
    tracker.assert_all_dropped_exactly_once(2);
}
