use super::{Cfg, DropTracker};
use crate::slot::repr::{EnumSlot, UnionSlot};
use crate::{
    u1, Even, KeyPiece, MapSlot, Odd, Parity, ParityMut, ParityRef, SecondaryMapSlot, Slot,
};
use core::mem::{size_of, ManuallyDrop};
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

/// Returns the only odd `u1`, which is one.
fn one() -> Odd<u1> {
    Odd::new(u1::MAX).unwrap()
}

#[test]
fn a_new_slot_has_the_generation_and_value_it_was_given() {
    let slot = Slot::<u32, &str, u8>::new(Parity::Odd(odd(3), "a"));
    assert_eq!(slot.generation(), 3);
    assert!(slot.is_odd());
    assert!(!slot.is_even());
    assert_eq!(slot.as_parity(), ParityRef::Odd(&odd(3), &"a"));

    let slot = Slot::<u32, &str, u8>::new(Parity::Even(even(4), 7));
    assert_eq!(slot.generation(), 4);
    assert!(slot.is_even());
    assert!(!slot.is_odd());
    assert_eq!(slot.as_parity(), ParityRef::Even(&even(4), &7));
}

#[test]
fn new_odd_and_new_even_match_new() {
    let slot = Slot::<u32, &str, u8>::new_odd(odd(3), "a");
    assert_eq!(slot.into_parity(), Parity::Odd(odd(3), "a"));
    let slot = Slot::<u32, &str, u8>::new_even(even(4), 7);
    assert_eq!(slot.into_parity(), Parity::Even(even(4), 7));
}

#[test]
fn as_parity_mut_changes_the_value_in_place() {
    let mut slot = Slot::<u32, i32, i32>::new(Parity::Odd(odd(1), 10));
    if let ParityMut::Odd(_, value) = slot.as_parity_mut() {
        *value += 5;
    }
    assert_eq!(slot.into_parity(), Parity::Odd(odd(1), 15));

    let mut slot = Slot::<u32, i32, i32>::new(Parity::Even(even(2), 20));
    if let ParityMut::Even(_, value) = slot.as_parity_mut() {
        *value += 5;
    }
    assert_eq!(slot.into_parity(), Parity::Even(even(2), 25));
}

#[test]
fn get_odd_only_matches_the_slots_own_generation() {
    let mut slot = Slot::<u32, i32, ()>::new(Parity::Odd(odd(5), 1));
    assert_eq!(slot.get_odd(odd(5)), Some(&1));
    assert_eq!(slot.get_odd(odd(3)), None);
    assert_eq!(slot.get_even(even(4)), None);
    assert_eq!(slot.get_even(even(6)), None);

    *slot.get_odd_mut(odd(5)).unwrap() = 2;
    assert!(slot.get_odd_mut(odd(7)).is_none());
    assert_eq!(slot.get_odd(odd(5)), Some(&2));
}

#[test]
fn get_even_only_matches_the_slots_own_generation() {
    let mut slot = Slot::<u32, (), i32>::new(Parity::Even(even(6), 1));
    assert_eq!(slot.get_even(even(6)), Some(&1));
    assert_eq!(slot.get_even(even(4)), None);
    assert_eq!(slot.get_odd(odd(5)), None);
    assert_eq!(slot.get_odd(odd(7)), None);

    *slot.get_even_mut(even(6)).unwrap() = 2;
    assert!(slot.get_even_mut(even(8)).is_none());
    assert_eq!(slot.get_even(even(6)), Some(&2));
}

#[test]
fn into_odd_and_into_even_hand_the_slot_back_when_the_generation_differs() {
    let slot = Slot::<u32, i32, i32>::new(Parity::Odd(odd(1), 10));
    let slot = slot.into_odd(odd(3)).unwrap_err();
    let slot = slot.into_even(even(2)).unwrap_err();
    assert_eq!(slot.into_odd(odd(1)).ok(), Some(10));

    let slot = Slot::<u32, i32, i32>::new(Parity::Even(even(2), 20));
    let slot = slot.into_even(even(4)).unwrap_err();
    let slot = slot.into_odd(odd(3)).unwrap_err();
    assert_eq!(slot.into_even(even(2)).ok(), Some(20));
}

#[test]
fn replace_swaps_the_whole_state_and_returns_the_old_one() {
    let mut slot = Slot::<u32, &str, Option<u32>>::new(Parity::Even(Even::ZERO, None));
    assert_eq!(
        slot.replace(Parity::Odd(odd(1), "a")),
        Parity::Even(Even::ZERO, None)
    );
    assert_eq!(slot.get_odd(odd(1)), Some(&"a"));
    assert_eq!(
        slot.replace(Parity::Even(even(2), Some(9))),
        Parity::Odd(odd(1), "a")
    );
    assert_eq!(slot.get_even(even(2)), Some(&Some(9)));
    assert_eq!(
        slot.replace(Parity::Odd(odd(3), "b")),
        Parity::Even(even(2), Some(9))
    );
    assert_eq!(slot.as_parity(), ParityRef::Odd(&odd(3), &"b"));
}

#[test]
fn set_odd_and_set_even_return_what_the_slot_held() {
    let mut slot = Slot::<u32, &str, Option<u32>>::new(Parity::Even(Even::ZERO, None));
    assert_eq!(slot.set_odd(odd(1), "a"), Parity::Even(Even::ZERO, None));
    assert_eq!(slot.as_parity(), ParityRef::Odd(&odd(1), &"a"));
    assert_eq!(slot.set_odd(odd(3), "b"), Parity::Odd(odd(1), "a"));
    assert_eq!(slot.set_even(even(4), Some(2)), Parity::Odd(odd(3), "b"));
    assert_eq!(slot.set_even(even(6), None), Parity::Even(even(4), Some(2)));
    assert_eq!(slot.as_parity(), ParityRef::Even(&even(6), &None));
}

#[test]
fn set_odd_and_set_even_drop_nothing_themselves() {
    let tracker = DropTracker::new();
    let mut slot = Slot::<u32, _, _>::new(Parity::Even(Even::ZERO, tracker.make_item()));
    let old = slot.set_odd(odd(1), tracker.make_item());
    tracker.assert_none_dropped();
    drop(old);
    let old = slot.set_even(even(2), tracker.make_item());
    assert_eq!(tracker.total_dropped(), 1);
    drop(old);
    drop(slot);
    tracker.assert_all_dropped_exactly_once(3);
}

#[test]
fn replace_unchecked_swaps_sides_and_returns_the_old_value() {
    let mut slot = Slot::<u32, &str, Option<u32>>::new_even(Even::ZERO, Some(4));
    assert_eq!(unsafe { slot.replace_even_unchecked(odd(1), "a") }, Some(4));
    assert_eq!(slot.as_parity(), ParityRef::Odd(&odd(1), &"a"));
    assert_eq!(unsafe { slot.replace_odd_unchecked(even(2), None) }, "a");
    assert_eq!(slot.as_parity(), ParityRef::Even(&even(2), &None));
}

#[test]
fn replace_unchecked_drops_nothing_itself() {
    let tracker = DropTracker::new();
    let mut slot = Slot::<u32, _, _>::new_even(Even::ZERO, tracker.make_item());
    let old = unsafe { slot.replace_even_unchecked(odd(1), tracker.make_item()) };
    tracker.assert_none_dropped();
    drop(old);
    let old = unsafe { slot.replace_odd_unchecked(even(2), tracker.make_item()) };
    assert_eq!(tracker.total_dropped(), 1);
    drop(old);
    drop(slot);
    tracker.assert_all_dropped_exactly_once(3);
}

#[test]
fn unchecked_access_reads_the_live_side() {
    let mut slot = Slot::<u32, i32, i32>::new(Parity::Odd(odd(1), 10));
    unsafe {
        assert_eq!(*slot.get_odd_unchecked(), 10);
        *slot.get_odd_unchecked_mut() = 11;
    }
    assert_eq!(slot.get_odd(odd(1)), Some(&11));

    let mut slot = Slot::<u32, i32, i32>::new(Parity::Even(even(2), 20));
    unsafe {
        assert_eq!(*slot.get_even_unchecked(), 20);
        *slot.get_even_unchecked_mut() = 21;
    }
    assert_eq!(slot.get_even(even(2)), Some(&21));
}

#[test]
fn dropping_a_slot_drops_only_the_live_side() {
    let tracker = DropTracker::new();
    drop(Slot::<u32, _, ()>::new(Parity::Odd(
        odd(1),
        tracker.make_item(),
    )));
    drop(Slot::<u32, (), _>::new(Parity::Even(
        even(2),
        tracker.make_item(),
    )));
    tracker.assert_all_dropped_exactly_once(2);
}

#[test]
fn values_taken_out_of_a_slot_are_dropped_once() {
    let tracker = DropTracker::new();

    let slot = Slot::<u32, _, ()>::new(Parity::Odd(odd(1), tracker.make_item()));
    let taken = slot.into_parity();
    tracker.assert_none_dropped();
    drop(taken);

    let slot = Slot::<u32, _, ()>::new(Parity::Odd(odd(1), tracker.make_item()));
    let slot = slot.into_odd(odd(3)).err().unwrap();
    let taken = slot.into_odd(odd(1)).ok().unwrap();
    assert_eq!(tracker.total_dropped(), 1);
    drop(taken);

    let mut slot = Slot::<u32, _, _>::new(Parity::Odd(odd(1), tracker.make_item()));
    let old = slot.replace(Parity::Even(even(2), tracker.make_item()));
    assert_eq!(tracker.total_dropped(), 2);
    drop(old);
    drop(slot);
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn a_clone_copies_the_live_side() {
    let slot = Slot::<u32, String, i32>::new(Parity::Odd(odd(7), String::from("a")));
    let copy = slot.clone();
    assert_eq!(copy.generation(), 7);
    assert_eq!(copy.get_odd(odd(7)).map(String::as_str), Some("a"));

    let slot = Slot::<u32, String, i32>::new(Parity::Even(even(8), 3));
    assert_eq!(slot.clone().get_even(even(8)), Some(&3));
}

#[test]
fn a_clone_and_its_original_are_each_dropped_once() {
    let tracker = DropTracker::new();
    let odd_slot = Slot::<u32, _, ()>::new_odd(odd(1), tracker.make_item());
    let even_slot = Slot::<u32, (), _>::new_even(even(2), tracker.make_item());
    let clones = (odd_slot.clone(), even_slot.clone());
    assert_eq!(tracker.total_made(), 4);
    tracker.assert_none_dropped();
    drop((odd_slot, even_slot, clones));
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn debug_shows_the_generation_and_the_live_value() {
    let slot = Slot::<u32, &str, i32>::new(Parity::Odd(odd(1), "a"));
    assert_eq!(format!("{slot:?}"), "Slot { generation: 1, value: \"a\" }");
    let slot = Slot::<u32, &str, i32>::new(Parity::Even(even(2), 5));
    assert_eq!(format!("{slot:?}"), "Slot { generation: 2, value: 5 }");
}

#[test]
fn as_parity_mut_changes_the_generation_but_not_the_parity() {
    let mut slot = Slot::<u32, &str, u8>::new(Parity::Odd(odd(1), "a"));
    if let ParityMut::Odd(generation, value) = slot.as_parity_mut() {
        *generation = odd(5);
        *value = "b";
    }
    assert_eq!(slot.as_parity(), ParityRef::Odd(&odd(5), &"b"));

    let mut slot = Slot::<u32, &str, u8>::new(Parity::Even(even(2), 7));
    if let ParityMut::Even(generation, value) = slot.as_parity_mut() {
        *generation = even(8);
        *value = 9;
    }
    assert_eq!(slot.as_parity(), ParityRef::Even(&even(8), &9));
}

#[test]
fn a_slot_with_a_u1_generation_hands_out_references_to_it() {
    let odd = Odd::new(u1::MAX).unwrap();
    let mut slot = Slot::<u1, &str, u8>::new_odd(odd, "a");
    assert_eq!(slot.as_parity(), ParityRef::Odd(&odd, &"a"));
    // An `Odd<u1>` takes no space, so this write leaves the generation at one.
    if let ParityMut::Odd(generation, value) = slot.as_parity_mut() {
        *generation = odd;
        *value = "b";
    }
    assert_eq!(slot.generation(), u1::MAX);
    assert_eq!(format!("{slot:?}"), "Slot { generation: 1, value: \"b\" }");
    assert_eq!(slot.into_parity(), Parity::Odd(odd, "b"));

    let mut slot = Slot::<u1, &str, u8>::new_even(Even::ZERO, 7);
    // An `Even<u1>` takes no space too, so this write leaves the generation at
    // zero.
    if let ParityMut::Even(generation, value) = slot.as_parity_mut() {
        *generation = Even::ZERO;
        *value = 9;
    }
    assert_eq!(slot.generation(), u1::MIN);
    assert_eq!(slot.as_parity(), ParityRef::Even(&Even::ZERO, &9));
    assert_eq!(slot.into_parity(), Parity::Even(Even::ZERO, 9));
}

#[test]
fn a_u1_slot_only_hands_out_the_side_it_holds() {
    let mut slot = Slot::<u1, &str, u8>::new_odd(one(), "a");
    assert!(slot.is_odd() && !slot.is_even());
    assert_eq!(slot.get_odd(one()), Some(&"a"));
    assert_eq!(slot.get_even(Even::ZERO), None);
    *slot.get_odd_mut(one()).unwrap() = "b";
    assert!(slot.get_even_mut(Even::ZERO).is_none());
    let slot = slot.into_even(Even::ZERO).unwrap_err();
    assert_eq!(slot.into_odd(one()).ok(), Some("b"));

    let mut slot = Slot::<u1, &str, u8>::new_even(Even::ZERO, 7);
    assert!(slot.is_even() && !slot.is_odd());
    assert_eq!(slot.get_even(Even::ZERO), Some(&7));
    assert_eq!(slot.get_odd(one()), None);
    *slot.get_even_mut(Even::ZERO).unwrap() = 8;
    assert!(slot.get_odd_mut(one()).is_none());
    let slot = slot.into_odd(one()).unwrap_err();
    assert_eq!(slot.into_even(Even::ZERO).ok(), Some(8));
}

#[test]
fn a_u1_slot_swaps_sides_and_returns_what_it_held() {
    let mut slot = Slot::<u1, &str, Option<u32>>::new(Parity::Even(Even::ZERO, None));
    assert_eq!(
        slot.replace(Parity::Odd(one(), "a")),
        Parity::Even(Even::ZERO, None)
    );
    assert_eq!(slot.set_odd(one(), "b"), Parity::Odd(one(), "a"));
    assert_eq!(slot.set_even(Even::ZERO, Some(1)), Parity::Odd(one(), "b"));
    assert_eq!(unsafe { slot.replace_even_unchecked(one(), "c") }, Some(1));
    unsafe {
        assert_eq!(*slot.get_odd_unchecked(), "c");
        *slot.get_odd_unchecked_mut() = "d";
    }
    assert_eq!(
        unsafe { slot.replace_odd_unchecked(Even::ZERO, Some(2)) },
        "d"
    );
    unsafe {
        assert_eq!(*slot.get_even_unchecked(), Some(2));
        *slot.get_even_unchecked_mut() = Some(3);
    }
    assert_eq!(slot.as_parity(), ParityRef::Even(&Even::ZERO, &Some(3)));
}

#[test]
fn a_u1_slot_drops_each_value_exactly_once() {
    let tracker = DropTracker::new();
    let mut slot = Slot::<u1, _, _>::new_even(Even::ZERO, tracker.make_item());
    let old = slot.set_odd(one(), tracker.make_item());
    tracker.assert_none_dropped();
    drop(old);
    let old = unsafe { slot.replace_odd_unchecked(Even::ZERO, tracker.make_item()) };
    drop(old);
    let old = unsafe { slot.replace_even_unchecked(one(), tracker.make_item()) };
    drop(old);
    let copy = slot.clone();
    let Err(slot) = slot.into_even(Even::ZERO) else {
        panic!("the slot holds a `T`, so `into_even` hands it back");
    };
    assert_eq!(tracker.total_dropped(), 3);
    drop((slot, copy));
    tracker.assert_all_dropped_exactly_once(5);

    // The same checks run for a slot whose `U` takes no space.
    let mut slot = Slot::<u1, _, ()>::new_odd(one(), tracker.make_item());
    let old = unsafe { slot.replace_odd_unchecked(Even::ZERO, ()) };
    drop(old);
    assert!(matches!(
        slot.set_odd(one(), tracker.make_item()),
        Parity::Even(_, ())
    ));
    let copy = slot.clone();
    drop((slot, copy));
    tracker.assert_all_dropped_exactly_once(8);
}

#[test]
fn a_u1_slot_clones_the_side_it_holds() {
    let slot = Slot::<u1, String, i32>::new_odd(one(), String::from("a"));
    assert_eq!(slot.clone().get_odd(one()).map(String::as_str), Some("a"));
    let slot = Slot::<u1, String, i32>::new_even(Even::ZERO, 3);
    assert_eq!(slot.clone().get_even(Even::ZERO), Some(&3));
    assert_eq!(format!("{slot:?}"), "Slot { generation: 0, value: 3 }");
}

#[test]
fn a_u1_slot_marks_its_side_in_a_bit_pattern_its_value_never_uses() {
    assert_eq!(size_of::<Slot<u1, Box<u64>, ()>>(), size_of::<Box<u64>>());
    assert_eq!(size_of::<Slot<u1, &str, ()>>(), size_of::<&str>());
    assert_eq!(size_of::<Slot<u1, String, u32>>(), size_of::<String>());
    assert_eq!(size_of::<Slot<u1, (u32, bool), u32>>(), 8);
    // Neither the `T` nor the `U` of the slots below has a bit pattern to
    // spare, so each of them is as large as a slot with a `u8` generation.
    assert_eq!(
        size_of::<Slot<u1, u64, u32>>(),
        size_of::<Slot<u8, u64, u32>>()
    );
    assert_eq!(
        size_of::<Slot<u1, u32, u32>>(),
        size_of::<Slot<u8, u32, u32>>()
    );
}

#[test]
fn the_slots_of_maps_with_a_u1_generation_use_the_spare_bit_patterns() {
    type U1Keys = Cfg<u32, u1>;
    assert_eq!(size_of::<MapSlot<String, U1Keys>>(), size_of::<String>());
    assert_eq!(
        size_of::<SecondaryMapSlot<Box<u64>, U1Keys>>(),
        size_of::<Box<u64>>()
    );
}

#[test]
fn integer_slots_keep_the_generation_next_to_the_value() {
    /// A `Beside` has the layout that a slot with an integer generation
    /// keeps, a generation next to a union of a `T` and a `U`.
    #[allow(dead_code)]
    struct Beside<G, T, U> {
        generation: G,
        value: Either<T, U>,
    }

    /// An `Either` holds a `T` or a `U` in the same place.
    #[allow(dead_code)]
    union Either<T, U> {
        t: ManuallyDrop<T>,
        u: ManuallyDrop<U>,
    }

    fn check<G: KeyPiece, T, U>() {
        assert_eq!(size_of::<Slot<G, T, U>>(), size_of::<Beside<G, T, U>>());
    }
    check::<u8, u8, u8>();
    check::<u32, u64, u32>();
    check::<u32, Box<u64>, ()>();
    check::<u64, u64, u64>();
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

#[test]
fn the_slot_representations_have_the_traits_that_slot_promises() {
    // `Slot` implements these traits by hand, so this checks that both of its
    // representations have them too, for every generation type.
    fn assert_traits<S: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe>() {}
    fn check<G: KeyPiece, T, U>()
    where
        T: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe,
        U: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe,
    {
        assert_traits::<UnionSlot<G, T, U>>();
        assert_traits::<EnumSlot<G, T, U>>();
    }
    check::<u1, String, u32>();
    check::<u32, String, u32>();
}
