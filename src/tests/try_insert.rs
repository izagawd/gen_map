use crate::{GenMap, InsertError, KeyConfig, MapConfig, SlotItem, Split};
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

#[test]
fn try_insert_works_like_insert_while_there_is_room() {
    let mut map = GenMap::new();
    let a = map.try_insert(1).unwrap();
    let b = map.try_insert(2).unwrap();
    assert_eq!(map[a], 1);
    assert_eq!(map[b], 2);
    assert_eq!(map.len(), 2);
}

#[test]
fn try_insert_hands_the_value_back_when_the_index_runs_out() {
    let mut map = GenMap::<String, Byte>::new_with_config();
    for i in 0..256 {
        map.try_insert(i.to_string()).unwrap();
    }
    match map.try_insert("late".to_string()) {
        Err(InsertError::IndexExhausted(value)) => assert_eq!(value, "late"),
        other => panic!("expected IndexExhausted, got {other:?}"),
    }
    assert_eq!(map.len(), 256);
    assert_eq!(map.slots_len(), 256);
}

#[test]
fn into_inner_takes_the_value_back() {
    assert_eq!(InsertError::<_, ()>::IndexExhausted(5).into_inner(), 5);
    assert_eq!(InsertError::StorageFull("x", ()).into_inner(), "x");
}

#[test]
fn the_error_does_not_need_debug_from_the_value() {
    struct Opaque;
    let text = std::format!("{:?}", InsertError::<_, ()>::IndexExhausted(Opaque));
    assert_eq!(text, "IndexExhausted(..)");
    let text = std::format!("{:?}", InsertError::StorageFull(Opaque, "why"));
    assert_eq!(text, "StorageFull(.., \"why\")");
    let text = std::format!("{}", InsertError::StorageFull(Opaque, "why"));
    assert!(text.contains("storage") && text.contains("why"));
}

#[test]
#[should_panic(expected = "cannot address more than 256 slots")]
fn insert_panics_when_the_index_runs_out() {
    let mut map = GenMap::<i32, Byte>::new_with_config();
    for i in 0..257 {
        map.insert(i);
    }
}
