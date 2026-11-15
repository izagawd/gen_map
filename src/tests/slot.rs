use super::Cfg;
use crate::{
    u1, DenseMapSlot, DenseSecondaryMapSlot, Even, KeyPiece, MapSlot, Odd, Parity, ParityUnion,
    SecondaryMapSlot, Slot,
};
use core::mem::size_of;
use core::panic::{RefUnwindSafe, UnwindSafe};
use std::boxed::Box;
use std::format;
use std::string::String;

fn odd(n: u32) -> Odd<u32> {
    Odd::new(n).unwrap()
}

fn even(n: u32) -> Even<u32> {
    Even::new(n).unwrap()
}

#[test]
fn a_default_slot_with_an_index_stores_the_largest_index() {
    let slot = Slot::<u32, &str, u8>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, u8::MAX));
    let slot = Slot::<u1, &str, usize>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, usize::MAX));
    let slot = Slot::<u8, &str, u1>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, u1::MAX));
}

#[test]
fn a_default_slot_without_an_index_stores_the_unit_value() {
    let slot = Slot::<u32, &str, ()>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, ()));
    let slot = Slot::<u1, &str, ()>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, ()));
}

#[test]
fn the_default_slot_of_every_map_holds_no_value() {
    type Byte = Cfg<u8, u8>;
    let slot = MapSlot::<&str, Byte>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, u8::MAX));
    let slot = DenseMapSlot::<Byte>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, u8::MAX));
    let slot = SecondaryMapSlot::<&str, Byte>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, ()));
    let slot = DenseSecondaryMapSlot::<Byte>::default();
    assert_eq!(slot.0.into_parity(), Parity::Even(Even::ZERO, ()));
}

#[test]
fn a_slot_hands_out_the_methods_of_its_parity_union() {
    let mut slot = Slot(ParityUnion::<u32, &str, u8>::new_odd(odd(3), "a"));
    assert_eq!(slot.generation(), 3);
    assert_eq!(slot.get_odd(odd(3)), Some(&"a"));

    // `set_even` takes `&mut self`, so this call changes the `ParityUnion`
    // inside the slot.
    assert_eq!(slot.set_even(even(4), 7), Parity::Odd(odd(3), "a"));
    assert_eq!(slot.0.into_parity(), Parity::Even(even(4), 7));
}

#[test]
fn a_clone_of_a_slot_holds_a_clone_of_its_value() {
    let slot = Slot(ParityUnion::<u32, String, u8>::new_odd(
        odd(7),
        String::from("a"),
    ));
    let copy = slot.clone();
    assert_eq!(copy.0.into_parity(), Parity::Odd(odd(7), String::from("a")));
    assert_eq!(slot.0.into_parity(), Parity::Odd(odd(7), String::from("a")));
}

#[test]
fn debug_shows_the_parity_union_of_a_slot() {
    let slot = Slot(ParityUnion::<u32, &str, u8>::new_odd(odd(1), "a"));
    assert_eq!(
        format!("{slot:?}"),
        "Slot(ParityUnion { generation: 1, value: \"a\" })"
    );
    let slot = Slot::<u32, &str, ()>::default();
    assert_eq!(
        format!("{slot:?}"),
        "Slot(ParityUnion { generation: 0, value: () })"
    );
}

#[test]
fn a_slot_is_as_large_as_its_parity_union() {
    fn check<G: KeyPiece, T, U>() {
        assert_eq!(
            size_of::<Slot<G, T, U>>(),
            size_of::<ParityUnion<G, T, U>>()
        );
    }
    check::<u1, String, u32>();
    check::<u1, Box<u64>, ()>();
    check::<u8, u8, u8>();
    check::<u32, u64, u32>();
    check::<usize, String, usize>();
}

#[test]
fn slots_keep_the_auto_traits_of_their_values_in_generic_code() {
    fn assert_traits<S: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe>() {}
    fn check<G: KeyPiece, T, U>()
    where
        T: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe,
        U: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe,
    {
        assert_traits::<Slot<G, T, U>>();
    }
    check::<u1, String, u32>();
    check::<u32, String, u32>();
}
