//! The map counts its values and links its free slots with the index type,
//! so neither the map nor a slot pays for an `Option` or a `usize`. These
//! sizes are for 64-bit targets.

use crate::{DefaultMapConfig, GenMap, GenMapConfig, GenSlotItem, MapConfig, MapSlot, Split};
use core::mem::size_of;
use std::vec::Vec;

struct Byte;

impl MapConfig for Byte {
    type KeyConfig = Split<u8, u8>;
}

impl<S: GenSlotItem> GenMapConfig<S> for Byte {
    type Storage = Vec<S>;
}

#[test]
fn the_map_is_a_vec_plus_two_indices() {
    // The default map holds a 24 byte `Vec`, a `u32` free list head and a
    // `u32` count.
    assert_eq!(size_of::<GenMap<u64>>(), 32);
    // A map with `u8` indices holds a 24 byte `Vec` and two bytes, and
    // padding rounds that up to the `Vec`'s alignment.
    assert_eq!(size_of::<GenMap<u64, Byte>>(), 32);
}

#[test]
fn a_free_link_takes_no_more_room_than_the_index() {
    // A default slot holds a `u32` generation next to either a value of at
    // most four bytes or a `u32` link.
    assert_eq!(size_of::<MapSlot<u32, DefaultMapConfig>>(), 8);
    assert_eq!(size_of::<MapSlot<(), DefaultMapConfig>>(), 8);
    // A slot of a map with `u8` indices holds a `u8` generation next to
    // either a one byte value or a `u8` link.
    assert_eq!(size_of::<MapSlot<u8, Byte>>(), 2);
}
