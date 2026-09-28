//! A storage with a fixed capacity and no `ReserveStorage`, to check that a
//! map works with such a storage and reports the storage's own error type.

use super::key_from_parts;
use crate::{
    FullError, GenMap, GenMapConfig, GenSlotItem, InsertError, InsertWithError, MapConfig,
    NewerWins, SecondaryInsertError, SecondaryMap, SecondaryMapConfig, SecondarySlotItem,
    SecondaryStorageError, SlotStorage, Split, StorageError,
};
use std::vec::Vec;

const CAP: usize = 4;

/// Holds at most `CAP` items and cannot grow on request.
struct Capped<S>(Vec<S>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CapReached;

impl<S> IntoIterator for Capped<S> {
    type Item = S;
    type IntoIter = std::vec::IntoIter<S>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

// SAFETY: this is a `Vec` that refuses pushes past `CAP`, and it behaves as
// the trait describes.
unsafe impl<S> SlotStorage for Capped<S> {
    type Item = S;
    type Error = CapReached;

    fn empty() -> Self {
        Capped(Vec::new())
    }

    fn with_capacity(_: usize) -> Self {
        Self::empty()
    }

    fn capacity(&self) -> usize {
        CAP
    }

    fn as_slice(&self) -> &[S] {
        &self.0
    }

    fn as_mut_slice(&mut self) -> &mut [S] {
        &mut self.0
    }

    fn ensure_room(&mut self, additional: usize) -> Result<(), CapReached> {
        if additional <= CAP - self.0.len() {
            Ok(())
        } else {
            Err(CapReached)
        }
    }

    fn try_push(&mut self, item: S) -> Result<(), S> {
        if self.0.len() < CAP {
            self.0.push(item);
            Ok(())
        } else {
            Err(item)
        }
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

struct Four;

impl MapConfig for Four {
    type KeyConfig = Split<u8, u8>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Four {
    type Storage = Capped<S>;
}

impl<S: SecondarySlotItem> SecondaryMapConfig<S> for Four {
    type ReplaceStrategy = NewerWins;
    type Storage = Capped<S>;
}

fn full_map() -> GenMap<i32, Four> {
    let mut map = GenMap::<i32, Four>::new_with_config();
    for i in 0..CAP as i32 {
        map.insert(i);
    }
    map
}

#[test]
fn the_storage_error_is_the_storages_own_type() {
    let mut map = full_map();
    let error: FullError<StorageError<i32, Four>> = map.vacant_entry().unwrap_err();
    assert_eq!(error, FullError::StorageFull(CapReached));
    assert_eq!(map.len(), CAP);
    assert_eq!(map.slots_len(), CAP);
}

#[test]
fn try_insert_hands_back_value_and_reason() {
    let mut map = full_map();
    let error = map.try_insert(99).unwrap_err();
    assert_eq!(error.into_parts(), (FullError::StorageFull(CapReached), 99));
    assert!(matches!(
        map.try_insert(99),
        Err(InsertError::StorageFull(99, CapReached))
    ));
}

#[test]
fn try_insert_with_key_does_not_call_the_closure_when_full() {
    let mut map = full_map();
    let mut called = false;
    let result = map.try_insert_with_key(|_| {
        called = true;
        Ok::<_, ()>(0)
    });
    assert_eq!(
        result,
        Err(InsertWithError::Full(FullError::StorageFull(CapReached)))
    );
    assert!(!called);
}

#[test]
fn a_freed_slot_is_reused_without_asking_the_storage() {
    let mut map = full_map();
    let key = map.iter().next().map(|(k, _)| k).unwrap();
    assert_eq!(map.remove(key), Some(0));
    let again = map.try_insert(10).unwrap();
    assert_eq!(again.idx(), key.idx());
    assert_ne!(again, key);
    assert_eq!(map[again], 10);
    assert_eq!(map.slots_len(), CAP);
}

#[test]
fn clone_works_on_a_fixed_storage() {
    let map = full_map();
    let copy = map.clone();
    assert_eq!(copy.len(), CAP);
    for (key, value) in &map {
        assert_eq!(copy[key], *value);
    }
}

#[test]
#[should_panic(expected = "cannot make room for more than 4 slots: CapReached")]
fn insert_panics_with_the_storages_reason() {
    let mut map = full_map();
    map.insert(99);
}

#[test]
fn a_secondary_map_hands_back_the_storages_own_error_and_adds_no_slots() {
    let mut map = SecondaryMap::<i32, Four>::new_with_config();
    assert_eq!(map.capacity(), CAP);
    let error: SecondaryInsertError<i32, SecondaryStorageError<i32, Four>> = map
        .insert(key_from_parts::<Split<u8, u8>>(9, 1), 9)
        .unwrap_err();
    assert!(matches!(
        error,
        SecondaryInsertError::StorageFull(9, CapReached)
    ));
    assert_eq!(map.slots_len(), 0);

    // Inserting at index 3 fills all four slots the storage has room for.
    assert_eq!(
        map.insert(key_from_parts::<Split<u8, u8>>(3, 1), 3)
            .unwrap(),
        None
    );
    assert_eq!(map.slots_len(), CAP);
    assert_eq!(map.len(), 1);
}

#[test]
fn a_secondary_map_clones_without_a_cloneable_storage() {
    // `Capped` does not implement `Clone`, and the map does not need it to.
    let mut map = SecondaryMap::<i32, Four>::new_with_config();
    let key = key_from_parts::<Split<u8, u8>>(2, 1);
    map.insert(key, 7).unwrap();
    let copy = map.clone();
    assert_eq!(copy.len(), 1);
    assert_eq!(copy.slots_len(), 3);
    assert_eq!(copy[key], 7);
}
