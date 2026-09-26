use crate::{
    FullError, GenMap, InsertError, InsertWithError, KeyConfig, MapConfig, SlotItem, Split,
};
use std::collections::TryReserveError;
use std::string::{String, ToString};
use std::vec::Vec;

struct Byte;

impl KeyConfig for Byte {
    type Idx = u8;
    type Gen = u8;
    type Layout = Split;
}

impl<S: SlotItem> MapConfig<S> for Byte {
    type KeyConfig = Self;
    type Storage = Vec<S>;
}

fn full_byte_map() -> GenMap<i32, Byte> {
    let mut map = GenMap::<i32, Byte>::new_with_config();
    for i in 0..256 {
        map.insert(i);
    }
    map
}

#[test]
fn vacant_entry_key_matches_the_inserted_key() {
    let mut map = GenMap::new();
    let entry = map.vacant_entry().unwrap();
    let promised = entry.key();
    let key = entry.insert("x");
    assert_eq!(promised, key);
    assert_eq!(map[key], "x");
    assert_eq!(map.len(), 1);
}

#[test]
fn dropped_vacant_entry_leaves_the_map_untouched() {
    let mut map: GenMap<i32> = GenMap::new();
    let promised = {
        let entry = map.vacant_entry().unwrap();
        entry.key()
    };
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 0);
    assert!(map.get(promised).is_none());

    // The next insert hands out the same key the dropped entry named.
    let key = map.insert(1);
    assert_eq!(key, promised);
}

#[test]
fn dropped_vacant_entry_keeps_a_freed_slot_on_the_free_list() {
    let mut map = GenMap::new();
    let a = map.insert(1);
    map.remove(a);

    let promised = map.vacant_entry().unwrap().key();
    assert_eq!(map.len(), 0);
    assert_eq!(map.slots_len(), 1);

    let b = map.insert(2);
    assert_eq!(b, promised);
    assert_eq!(b.idx(), a.idx());
    assert_ne!(b, a);
    assert!(map.get(a).is_none());
}

#[test]
fn vacant_entry_reports_index_exhausted() {
    let mut map = full_byte_map();
    assert_eq!(map.vacant_entry().err(), Some(FullError::IndexExhausted));
    assert_eq!(map.len(), 256);
}

#[test]
fn try_insert_with_key_hands_back_the_closure_error() {
    let mut map: GenMap<i32> = GenMap::new();
    let result = map.try_insert_with_key(|_| Err::<i32, _>("nope"));
    assert_eq!(result, Err(InsertWithError::Rejected("nope")));
    assert_eq!(map.len(), 0);
}

#[test]
fn try_insert_with_key_reports_full_without_calling_the_closure() {
    let mut map = full_byte_map();
    let mut called = false;
    let result = map.try_insert_with_key(|_| {
        called = true;
        Ok::<_, ()>(0)
    });
    assert_eq!(
        result,
        Err(InsertWithError::Full(FullError::IndexExhausted))
    );
    assert!(!called);
    assert_eq!(map.len(), 256);
}

#[test]
fn flatten_folds_full_into_the_callers_error() {
    #[derive(Debug, PartialEq)]
    enum MyError {
        Full(FullError<TryReserveError>),
        Bad,
    }

    impl From<FullError<TryReserveError>> for MyError {
        fn from(e: FullError<TryReserveError>) -> Self {
            MyError::Full(e)
        }
    }

    let mut map = full_byte_map();
    let result = map
        .try_insert_with_key(|_| Ok::<_, MyError>(0))
        .map_err(InsertWithError::flatten);
    assert_eq!(result, Err(MyError::Full(FullError::IndexExhausted)));

    let mut map: GenMap<i32> = GenMap::new();
    let result = map
        .try_insert_with_key(|_| Err::<i32, _>(MyError::Bad))
        .map_err(InsertWithError::flatten);
    assert_eq!(result, Err(MyError::Bad));
}

#[test]
fn full_error_converts_into_insert_with_error() {
    let e: InsertWithError<String, &str> = FullError::StorageFull("why").into();
    assert_eq!(e, InsertWithError::Full(FullError::StorageFull("why")));
    assert!(e.is_full());
    assert_eq!(e.rejected(), None);

    let e = InsertWithError::<String, ()>::Rejected("x".to_string());
    assert!(!e.is_full());
    assert_eq!(e.rejected(), Some("x".to_string()));
}

#[test]
fn insert_error_kind_and_parts_agree_with_the_variant() {
    let e = InsertError::<_, ()>::IndexExhausted(5);
    assert_eq!(e.kind(), FullError::IndexExhausted);
    assert_eq!(e.into_parts(), (FullError::IndexExhausted, 5));

    let e = InsertError::StorageFull("x", "why");
    assert_eq!(e.kind(), FullError::StorageFull(&"why"));
    assert_eq!(e.into_parts(), (FullError::StorageFull("why"), "x"));
}

#[test]
fn full_error_as_ref_borrows_the_storage_error() {
    let e = FullError::StorageFull("why".to_string());
    assert_eq!(e.as_ref(), FullError::StorageFull(&"why".to_string()));
    assert_eq!(
        FullError::<String>::IndexExhausted.as_ref(),
        FullError::IndexExhausted
    );
}

#[test]
fn vacant_entry_debug_prints_the_key() {
    let mut map = GenMap::<i32>::new();
    map.insert(1);
    let entry = map.vacant_entry().unwrap();
    let text = std::format!("{entry:?}");
    assert!(text.starts_with("VacantEntry"));
    assert!(text.contains(&std::format!("{:?}", entry.key())));
}
