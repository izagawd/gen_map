//! Configs that restrict the types of the values a map can hold.

use crate::{
    DenseGenMap, DenseGenMapConfig, DenseMapSlot, DenseSecondaryMap, DenseSecondaryMapConfig,
    DenseSecondaryMapSlot, GenMap, GenMapConfig, KeepLen, MapConfig, MapKey, MapSlot, NewerWins,
    PairVec, SecondaryMap, SecondaryMapConfig, SecondaryMapSlot, Split,
};
use std::vec::Vec;

/// Only the tests in this file implement this trait.
trait Component {}

#[derive(Debug, PartialEq)]
struct Position(i32);

impl Component for Position {}

/// Maps with this config can only hold values that implement [`Component`].
struct Components;

impl MapConfig for Components {
    type KeyConfig = Split<u32, u32>;
}

impl<T: Component> GenMapConfig<T> for Components {
    type Len = KeepLen;
    type Storage = Vec<MapSlot<T, Self>>;
}

impl<T: Component> SecondaryMapConfig<T> for Components {
    type ReplaceStrategy = NewerWins;
    type Len = KeepLen;
    type Storage = Vec<SecondaryMapSlot<T, Self>>;
}

impl<T: Component> DenseGenMapConfig<T> for Components {
    type SlotStorage = Vec<DenseMapSlot<Self>>;
    type PairStorage = PairVec<MapKey<Self>, T>;
}

impl<T: Component> DenseSecondaryMapConfig<T> for Components {
    type ReplaceStrategy = NewerWins;
    type SlotStorage = Vec<DenseSecondaryMapSlot<Self>>;
    type PairStorage = PairVec<MapKey<Self>, T>;
}

#[cfg(feature = "std")]
impl<T: Component> crate::SparseSecondaryMapConfig<T> for Components {
    type ReplaceStrategy = NewerWins;
}

#[test]
fn a_config_can_bound_the_value_type_by_a_trait() {
    let mut map = GenMap::<Position, Components>::new_with_config();
    let key = map.insert(Position(1));
    assert_eq!(map[key], Position(1));

    let mut dense = DenseGenMap::<Position, Components>::new_with_config();
    let dense_key = dense.insert(Position(2));
    assert_eq!(dense[dense_key], Position(2));

    let mut secondary = SecondaryMap::<Position, Components>::new_with_config();
    secondary.insert(key, Position(3));
    assert_eq!(secondary[key], Position(3));

    let mut dense_secondary = DenseSecondaryMap::<Position, Components>::new_with_config();
    dense_secondary.insert(key, Position(4));
    assert_eq!(dense_secondary[key], Position(4));
}

#[cfg(feature = "std")]
#[test]
fn a_config_can_bound_the_value_type_of_a_sparse_secondary_map() {
    let mut map = GenMap::<Position, Components>::new_with_config();
    let key = map.insert(Position(1));

    let mut sparse = crate::SparseSecondaryMap::<Position, Components>::new_with_config();
    sparse.insert(key, Position(2));
    assert_eq!(sparse[key], Position(2));
}

/// A `GenMap` with this config can only hold `u32` values.
struct OnlyU32;

impl MapConfig for OnlyU32 {
    type KeyConfig = Split<u32, u32>;
}

impl GenMapConfig<u32> for OnlyU32 {
    type Len = KeepLen;
    type Storage = Vec<MapSlot<u32, Self>>;
}

#[test]
fn a_config_can_work_with_a_single_value_type() {
    // The config only implements `GenMapConfig<u32>`, so the compiler infers
    // that the values are `u32`. If it did not, the `7` would be an `i32`, and
    // this test would not compile, because the config has no impl for `i32`.
    let mut map = GenMap::<_, OnlyU32>::new_with_config();
    let key = map.insert(7);
    assert_eq!(map[key], 7);
}
