use super::DropTracker;
use crate::{DefaultMapConfig, Even, GenMap, MapSlot, Odd, Parity, Slot};
use std::format;
use std::string::String;

fn odd(n: u32) -> Odd<u32> {
    Odd::new(n).unwrap()
}

fn even(n: u32) -> Even<u32> {
    Even::new(n).unwrap()
}

#[test]
fn a_new_slot_has_the_generation_and_value_it_was_given() {
    let slot = Slot::<u32, &str, u8>::new(Parity::Odd(odd(3), "a"));
    assert_eq!(slot.generation(), 3);
    assert!(slot.is_odd());
    assert!(!slot.is_even());
    assert_eq!(slot.as_parity(), Parity::Odd(odd(3), &"a"));

    let slot = Slot::<u32, &str, u8>::new(Parity::Even(even(4), 7));
    assert_eq!(slot.generation(), 4);
    assert!(slot.is_even());
    assert!(!slot.is_odd());
    assert_eq!(slot.as_parity(), Parity::Even(even(4), &7));
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
    if let Parity::Odd(_, value) = slot.as_parity_mut() {
        *value += 5;
    }
    assert_eq!(slot.into_parity(), Parity::Odd(odd(1), 15));

    let mut slot = Slot::<u32, i32, i32>::new(Parity::Even(even(2), 20));
    if let Parity::Even(_, value) = slot.as_parity_mut() {
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
    assert_eq!(slot.as_parity(), Parity::Odd(odd(3), &"b"));
}

#[test]
fn set_odd_and_set_even_return_what_the_slot_held() {
    let mut slot = Slot::<u32, &str, Option<u32>>::new(Parity::Even(Even::ZERO, None));
    assert_eq!(slot.set_odd(odd(1), "a"), Parity::Even(Even::ZERO, None));
    assert_eq!(slot.as_parity(), Parity::Odd(odd(1), &"a"));
    assert_eq!(slot.set_odd(odd(3), "b"), Parity::Odd(odd(1), "a"));
    assert_eq!(slot.set_even(even(4), Some(2)), Parity::Odd(odd(3), "b"));
    assert_eq!(slot.set_even(even(6), None), Parity::Even(even(4), Some(2)));
    assert_eq!(slot.as_parity(), Parity::Even(even(6), &None));
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
    assert_eq!(slot.as_parity(), Parity::Odd(odd(1), &"a"));
    assert_eq!(unsafe { slot.replace_odd_unchecked(even(2), None) }, "a");
    assert_eq!(slot.as_parity(), Parity::Even(even(2), &None));
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
fn a_slot_can_hold_zero_sized_values() {
    let mut slot = Slot::<u8, (), ()>::new_even(Even::ZERO, ());
    assert_eq!(
        slot.set_odd(Odd::new(1).unwrap(), ()),
        Parity::Even(Even::ZERO, ())
    );
    assert_eq!(slot.into_parity(), Parity::Odd(Odd::new(1).unwrap(), ()));
}

#[test]
fn map_slot_is_a_slot_of_the_key_types() {
    fn same(slot: MapSlot<u8, DefaultMapConfig>) -> Slot<u32, u8, Option<u32>> {
        slot
    }
    let slot = same(Slot::new_odd(odd(1), 7));
    assert_eq!(slot.get_odd(odd(1)), Some(&7));
}

#[test]
fn debug_shows_the_generation_and_the_live_value() {
    let slot = Slot::<u32, &str, i32>::new(Parity::Odd(odd(1), "a"));
    assert_eq!(format!("{slot:?}"), "Slot { generation: 1, value: \"a\" }");
    let slot = Slot::<u32, &str, i32>::new(Parity::Even(even(2), 5));
    assert_eq!(format!("{slot:?}"), "Slot { generation: 2, value: 5 }");
}

#[test]
fn a_map_key_finds_the_odd_side_of_a_slot_with_its_generation() {
    let mut map = GenMap::new();
    let key = map.insert("a");
    let mut slot = Slot::<u32, u64, ()>::new(Parity::Even(Even::ZERO, ()));
    assert!(slot.get_odd(key.generation()).is_none());

    slot.replace(Parity::Odd(key.generation(), 10));
    assert_eq!(slot.get_odd(key.generation()), Some(&10));

    map.remove(key);
    let newer = map.insert("b");
    assert!(slot.get_odd(newer.generation()).is_none());
}
