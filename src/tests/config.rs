//! Configs that put bounds on the slots they support, through the value
//! type the slots hold.

use crate::{DefaultKeyConfig, GenMap, GenMapConfig, GenSlotItem, Key, MapConfig};
use std::vec::Vec;

/// A trait only these tests implement.
trait Component {}

#[derive(Debug, PartialEq)]
struct Position(i32);

impl Component for Position {}

/// Only supports values that implement `Component`.
struct Components;

impl MapConfig for Components {
    type KeyConfig = DefaultKeyConfig;
}

impl<S: GenSlotItem> GenMapConfig<S> for Components
where
    S::Value: Component,
{
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

impl MapConfig for OnlyU32 {
    type KeyConfig = DefaultKeyConfig;
}

impl<S: GenSlotItem<Value = u32>> GenMapConfig<S> for OnlyU32 {
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
