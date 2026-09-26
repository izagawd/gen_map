//! Configs that put bounds on the slots they support, through the value
//! type the slots hold.

use crate::{DefaultKeyConfig, GenMap, Key, KeyConfig, MapConfig, SlotItem, Split};
use std::vec::Vec;

/// A trait only these tests implement.
trait Component {}

#[derive(Debug, PartialEq)]
struct Position(i32);

impl Component for Position {}

/// Only supports values that implement `Component`.
struct Components;

impl<S: SlotItem> MapConfig<S> for Components
where
    S::Value: Component,
{
    type KeyConfig = DefaultKeyConfig;
    type Storage = Vec<S>;
}

#[test]
fn a_config_can_bound_the_value_by_any_trait() {
    let mut map = GenMap::<Position, Components>::new_with_config();
    let key = map.insert(Position(3));
    assert_eq!(map[key], Position(3));
}

/// Only supports `u32` values.
struct OnlyU32;

impl<S: SlotItem<Value = u32>> MapConfig<S> for OnlyU32 {
    type KeyConfig = DefaultKeyConfig;
    type Storage = Vec<S>;
}

#[test]
fn a_config_can_support_a_single_value_type() {
    let mut map = GenMap::<u32, OnlyU32>::new_with_config();
    let key = map.insert(7);
    assert_eq!(map[key], 7);

    // Configs that name the same key config share a key type.
    let mut other = GenMap::<Position, Components>::new_with_config();
    let keys: [Key; 2] = [key, other.insert(Position(1))];
    assert_eq!(keys[0], keys[1]);
}

/// Names the key config for a value type.
trait KeysFor {
    type Keys: KeyConfig;
}

struct ByteKeys;

impl KeyConfig for ByteKeys {
    type Idx = u8;
    type Gen = u8;
    type Layout = Split;
}

impl KeysFor for u8 {
    type Keys = ByteKeys;
}

impl KeysFor for u64 {
    type Keys = DefaultKeyConfig;
}

/// Takes its key config from the value type.
struct PerValue;

impl<S: SlotItem> MapConfig<S> for PerValue
where
    S::Value: KeysFor,
{
    type KeyConfig = <S::Value as KeysFor>::Keys;
    type Storage = Vec<S>;
}

#[test]
fn a_key_config_can_depend_on_the_value_type() {
    let mut small = GenMap::<u8, PerValue>::new_with_config();
    let mut large = GenMap::<u64, PerValue>::new_with_config();
    let small_key: Key<ByteKeys> = small.insert(1);
    let large_key: Key = large.insert(2);
    assert_eq!(core::mem::size_of_val(&small_key), 2);
    assert_eq!(core::mem::size_of_val(&large_key), 8);
    assert_eq!((small[small_key], large[large_key]), (1, 2));
}
