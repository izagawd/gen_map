//! Tests for `SingleVec` on its own. The maps keep their slots in it under the
//! default config, so the map tests cover it as well.

use crate::tests::model::Rng;
use crate::tests::{Bomb, DropTracker};
use crate::{ReserveError, SingleVec, SingleVecIntoIter, SingleVecRawParts, SliceStorage};
use core::mem::ManuallyDrop;
use core::ptr::NonNull;
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::string::{String, ToString};
use std::vec::Vec;

#[test]
fn pushes_and_pops_work_like_a_vec() {
    let mut items = SingleVec::<_>::new();
    assert!(items.is_empty());
    for i in 0..10u32 {
        items.push(i.to_string());
    }
    assert_eq!(items.len(), 10);
    assert!(items.iter().cloned().eq((0..10).map(|i| i.to_string())));
    assert_eq!(items[3], "3");
    assert_eq!(items.pop(), Some(String::from("9")));
    assert_eq!(items.len(), 9);
}

#[test]
fn the_slice_it_derefs_to_changes_the_items_in_place() {
    let mut items: SingleVec<u32, u16> = (0..4).collect();
    items.swap(0, 3);
    items[1] = 100;
    assert_eq!(items[..], [3, 100, 2, 0]);
    assert_eq!(items.as_slice(), [3, 100, 2, 0]);
}

#[test]
fn growing_moves_every_item_into_the_new_buffer() {
    let mut items = SingleVec::<_, u32>::with_capacity(3);
    let capacity = items.capacity();
    assert!(capacity >= 3);
    // Pushing as many items as there is room for needs no new buffer.
    for i in 0..capacity as u64 {
        items.push(i);
    }
    assert_eq!(items.capacity(), capacity);

    for i in capacity as u64..100 {
        items.push(i);
    }
    assert!(items.capacity() >= 100);
    assert!(items.iter().copied().eq(0..100));
}

#[test]
fn ensure_room_makes_room_for_that_many_pushes() {
    let mut items = SingleVec::<String, u16>::new();
    items.push(String::new());
    items.ensure_room(20).unwrap();
    let capacity = items.capacity();
    assert!(capacity >= 21);
    for _ in 1..21 {
        assert!(items.try_push(String::new()).is_ok());
    }
    assert_eq!(items.capacity(), capacity);
}

#[test]
fn room_that_no_buffer_can_have_is_an_error() {
    let mut items = SingleVec::<u64>::new();
    items.push(1);
    assert_eq!(
        items.ensure_room(usize::MAX),
        Err(ReserveError::CapacityOverflow)
    );
    // A buffer with room for this many `u64`s would take more than
    // `isize::MAX` bytes.
    assert_eq!(
        items.ensure_room(usize::MAX / 8),
        Err(ReserveError::CapacityOverflow)
    );
    assert_eq!(items[..], [1]);
}

#[test]
fn the_length_type_caps_how_many_items_fit() {
    let mut items = SingleVec::<u32, u8>::new();
    for i in 0..255 {
        items.push(i);
    }
    assert_eq!(items.capacity(), 255);
    assert_eq!(items.try_push(255), Err(255));
    assert_eq!(items.ensure_room(1), Err(ReserveError::CapacityOverflow));
    assert_eq!(items.len(), 255);
    assert_eq!(items.pop(), Some(254));
    assert!(items.try_push(254).is_ok());
    assert!(items.into_iter().eq(0..255));
}

#[test]
fn a_capacity_the_length_type_cannot_count_panics() {
    assert_eq!(SingleVec::<u8, u8>::with_capacity(255).capacity(), 255);
    assert!(catch_unwind(|| SingleVec::<u8, u8>::with_capacity(256)).is_err());
    assert!(catch_unwind(|| SingleVec::<(), u8>::with_capacity(256)).is_err());
}

#[test]
fn items_that_take_no_space_need_no_buffer() {
    let mut units = SingleVec::<()>::new();
    assert_eq!(units.capacity(), usize::MAX);
    for _ in 0..1000 {
        units.push(());
    }
    assert_eq!(units.len(), 1000);
    assert!(units.ensure_room(usize::MAX - 1000).is_ok());
    assert_eq!(
        units.ensure_room(usize::MAX - 999),
        Err(ReserveError::CapacityOverflow)
    );
    assert_eq!(units.pop(), Some(()));
    assert_eq!(units.into_iter().count(), 999);

    // The length type still caps how many of them fit.
    let mut units = SingleVec::<(), u8>::new();
    assert_eq!(units.capacity(), 255);
    for _ in 0..255 {
        units.push(());
    }
    assert_eq!(units.try_push(()), Err(()));
}

#[test]
fn every_item_is_dropped_exactly_once() {
    let tracker = DropTracker::new();
    let mut items = SingleVec::<_, u16>::new();
    for _ in 0..10 {
        items.push(tracker.make_item());
    }
    drop(items.pop());
    // The clone makes a new item for each of the 9 items left, and dropping
    // the clone drops those.
    drop(items.clone());
    items.clear();
    items.push(tracker.make_item());
    drop(items);
    tracker.assert_all_dropped_exactly_once(tracker.total_made());
    assert_eq!(tracker.total_made(), 20);
}

#[test]
fn clear_drops_every_item_and_empties_the_vec_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let mut items = SingleVec::<_, u8>::new();
    for i in 0..6 {
        items.push(Bomb::new(&tracker, i == 2));
    }
    assert!(catch_unwind(AssertUnwindSafe(|| items.clear())).is_err());
    assert!(items.is_empty());
    items.push(Bomb::new(&tracker, false));
    assert_eq!(items.len(), 1);
    drop(items);
    tracker.assert_all_dropped_exactly_once(7);
}

#[test]
fn dropping_the_vec_drops_every_item_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let mut items = SingleVec::<_>::new();
    for i in 0..5 {
        items.push(Bomb::new(&tracker, i == 1));
    }
    assert!(catch_unwind(AssertUnwindSafe(|| drop(items))).is_err());
    tracker.assert_all_dropped_exactly_once(5);
}

#[test]
fn into_iter_yields_from_both_ends_and_drops_the_items_it_did_not_yield() {
    let tracker = DropTracker::new();
    let items: SingleVec<_, u32> = (0..6).map(|_| tracker.make_item()).collect();
    let mut iter = items.into_iter();
    assert_eq!(iter.len(), 6);
    assert!(iter.next().is_some());
    assert!(iter.next_back().is_some());
    assert_eq!(iter.len(), 4);
    drop(iter);
    tracker.assert_all_dropped_exactly_once(6);
}

#[test]
fn into_iter_drops_the_rest_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let items: SingleVec<_> = (0..4).map(|i| Bomb::new(&tracker, i == 2)).collect();
    let mut iter = items.into_iter();
    drop(iter.next());
    assert!(catch_unwind(AssertUnwindSafe(|| drop(iter))).is_err());
    tracker.assert_all_dropped_exactly_once(4);
}

#[test]
fn clone_and_debug_show_the_same_items() {
    let items: SingleVec<&str, u8> = ["a", "b"].into_iter().collect();
    let copy = items.clone();
    assert_eq!(copy[..], items[..]);
    assert_eq!(format!("{items:?}"), r#"["a", "b"]"#);
    assert!(SingleVec::<u8>::default().is_empty());
}

#[test]
fn a_single_vec_and_its_iterator_can_move_between_threads() {
    fn assert_send_and_sync<T: Send + Sync>() {}
    assert_send_and_sync::<SingleVec<String, u32>>();
    assert_send_and_sync::<SingleVecIntoIter<String, u32>>();
}

#[test]
fn a_single_vec_agrees_with_a_vec() {
    let (seeds, steps) = if cfg!(miri) { (2, 200) } else { (16, 2000) };
    for seed in 0..seeds {
        let mut rng = Rng(seed);
        let mut items = SingleVec::<String, u16>::new();
        let mut expected = Vec::new();
        for _ in 0..steps {
            match rng.below(10) {
                0..=4 => {
                    let item = rng.next().to_string();
                    items.push(item.clone());
                    expected.push(item);
                }
                5 | 6 => assert_eq!(items.pop(), expected.pop()),
                7 => {
                    let additional = rng.below(20);
                    items.ensure_room(additional).unwrap();
                    assert!(items.capacity() >= items.len() + additional);
                }
                8 if !expected.is_empty() => {
                    let i = rng.below(expected.len());
                    items[i].push('x');
                    expected[i].push('x');
                }
                _ if rng.chance(10) => {
                    items.clear();
                    expected.clear();
                }
                _ => {}
            }
            assert_eq!(items[..], expected[..]);
        }
        assert_eq!(items.into_iter().collect::<Vec<_>>(), expected);
    }
}

#[test]
fn raw_parts_give_back_the_same_vec() {
    let tracker = DropTracker::new();
    let mut items = SingleVec::<_, u16>::with_capacity(8);
    for _ in 0..5 {
        items.push(tracker.make_item());
    }
    let parts = items.into_raw_parts();
    assert_eq!((parts.capacity, parts.len), (8, 5));
    tracker.assert_none_dropped();
    let items = unsafe { SingleVec::from_raw_parts(parts) };
    assert_eq!((items.capacity(), items.len()), (8, 5));
    drop(items);
    tracker.assert_all_dropped_exactly_once(5);
}

#[test]
fn a_vec_can_take_over_the_buffer_of_a_std_vec() {
    let tracker = DropTracker::new();
    let mut vec = ManuallyDrop::new((0..3).map(|_| tracker.make_item()).collect::<Vec<_>>());
    let parts = SingleVecRawParts {
        pointer: NonNull::new(vec.as_mut_ptr()).unwrap(),
        capacity: vec.capacity(),
        len: vec.len(),
    };
    let mut items: SingleVec<_> = unsafe { SingleVec::from_raw_parts(parts) };
    // Growing frees the buffer of the `Vec`.
    for _ in 0..10 {
        items.push(tracker.make_item());
    }
    drop(items);
    tracker.assert_all_dropped_exactly_once(13);
}

#[test]
fn raw_parts_of_items_that_take_no_space_keep_their_count() {
    let mut units = SingleVec::<(), u8>::new();
    for _ in 0..3 {
        units.push(());
    }
    let mut parts = units.into_raw_parts();
    assert_eq!((parts.capacity, parts.len), (255, 3));
    // Items that take no space need no buffer, so any capacity of at least
    // `len` follows the rules, and the vec's capacity is still the largest
    // `u8`.
    parts.capacity = 3;
    let units = unsafe { SingleVec::from_raw_parts(parts) };
    assert_eq!((units.capacity(), units.len()), (255, 3));
}
