//! Tests for `PairVec` on its own. The dense maps use it as the pair storage
//! of the default config, so the dense map tests cover it as well.

use super::model::Rng;
use super::{Bomb, DropTracker};
use crate::{PairStorage, PairVec, PairVecError, PairVecIntoIter};
use core::alloc::Layout;
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::string::{String, ToString};
use std::vec::Vec;

#[test]
fn pushes_and_pops_keep_the_two_slices_in_step() {
    let mut pairs = PairVec::new();
    assert!(pairs.is_empty());
    for i in 0..10u32 {
        pairs.push(i, i.to_string());
    }
    assert_eq!(pairs.len(), 10);
    let (firsts, seconds) = pairs.slices();
    assert!(firsts.iter().copied().eq(0..10));
    assert!(seconds.iter().cloned().eq((0..10).map(|i| i.to_string())));

    assert_eq!(pairs.pop(), Some((9, String::from("9"))));
    assert_eq!(pairs.pop(), Some((8, String::from("8"))));
    assert_eq!(pairs.len(), 8);
    assert_eq!(pairs.second_slice().len(), 8);
}

#[test]
fn the_mutable_slices_change_the_pairs_in_place() {
    let mut pairs: PairVec<u32, u64> = (0..4).map(|i| (i, u64::from(i) * 10)).collect();
    let (firsts, seconds) = pairs.slices_mut();
    firsts.swap(0, 3);
    seconds.swap(0, 3);
    pairs.first_slice_mut()[1] = 100;
    pairs.second_slice_mut()[1] = 1000;
    assert_eq!(pairs.first_slice(), [3, 100, 2, 0]);
    assert_eq!(pairs.second_slice(), [30, 1000, 20, 0]);
}

#[test]
fn growing_moves_every_pair_into_the_new_buffers() {
    let mut pairs = PairVec::with_capacity(3);
    let capacity = pairs.capacity();
    assert!(capacity >= 3);
    // Pushing as many pairs as there is room for needs no new buffers.
    for i in 0..capacity as u64 {
        pairs.push(i, [i as u8; 3]);
    }
    assert_eq!(pairs.capacity(), capacity);

    for i in capacity as u64..100 {
        pairs.push(i, [i as u8; 3]);
    }
    assert!(pairs.capacity() >= 100);
    assert!(pairs.first_slice().iter().copied().eq(0..100));
    assert!(pairs
        .second_slice()
        .iter()
        .copied()
        .eq((0..100).map(|i| [i as u8; 3])));
}

#[test]
fn try_push_appends_a_pair_like_push() {
    let mut pairs = PairVec::new();
    for i in 0..10u8 {
        assert_eq!(pairs.try_push(i, char::from(b'a' + i)), Ok(()));
    }
    assert!(pairs.first_slice().iter().copied().eq(0..10));
    assert!(pairs
        .second_slice()
        .iter()
        .copied()
        .eq("abcdefghij".chars()));
}

#[test]
fn ensure_room_makes_room_for_that_many_pushes() {
    let mut pairs = PairVec::<u8, String>::new();
    pairs.push(0, String::new());
    pairs.ensure_room(20).unwrap();
    let capacity = pairs.capacity();
    assert!(capacity >= 21);
    for i in 1..21 {
        assert!(pairs.try_push(i, String::new()).is_ok());
    }
    assert_eq!(pairs.capacity(), capacity);
}

#[test]
fn room_that_no_buffer_can_have_is_an_error() {
    let mut pairs = PairVec::<u64, u8>::new();
    pairs.push(1, 2);
    assert_eq!(
        pairs.ensure_room(usize::MAX),
        Err(PairVecError::CapacityOverflow)
    );
    // A buffer with room for this many `u64`s would take more than
    // `isize::MAX` bytes.
    assert_eq!(
        pairs.ensure_room(usize::MAX / 8),
        Err(PairVecError::CapacityOverflow)
    );
    assert_eq!(pairs.slices(), (&[1][..], &[2][..]));
}

#[test]
fn the_error_says_why_there_is_no_room() {
    assert_eq!(
        format!("{}", PairVecError::CapacityOverflow),
        "the pair storage cannot hold that many pairs"
    );
    assert_eq!(
        format!("{}", PairVecError::AllocError(Layout::new::<u64>())),
        "the allocator could not allocate 8 bytes for a buffer"
    );
}

#[test]
fn slices_of_items_that_take_no_space_need_no_buffer() {
    let mut units = PairVec::<(), ()>::new();
    assert_eq!(units.capacity(), usize::MAX);
    for _ in 0..1000 {
        units.push((), ());
    }
    assert_eq!(units.len(), 1000);
    assert_eq!(units.capacity(), usize::MAX);
    assert!(units.ensure_room(usize::MAX - 1000).is_ok());
    assert_eq!(
        units.ensure_room(usize::MAX - 999),
        Err(PairVecError::CapacityOverflow)
    );
    assert_eq!(units.pop(), Some(((), ())));
    assert_eq!(units.into_iter().count(), 999);
}

#[test]
fn a_slice_of_items_that_take_no_space_grows_with_the_other_slice() {
    let mut pairs = PairVec::<(), u32>::new();
    let mut flipped = PairVec::<u32, ()>::new();
    for i in 0..100 {
        pairs.push((), i);
        flipped.push(i, ());
    }
    assert!((100..usize::MAX).contains(&pairs.capacity()));
    assert!(pairs.second_slice().iter().copied().eq(0..100));
    assert!(flipped.first_slice().iter().copied().eq(0..100));
    assert_eq!(pairs.first_slice().len(), 100);
    assert_eq!(flipped.second_slice().len(), 100);
    assert_eq!(pairs.pop(), Some(((), 99)));
    assert_eq!(flipped.into_iter().next_back(), Some((99, ())));
}

#[test]
fn every_item_is_dropped_exactly_once() {
    let tracker = DropTracker::new();
    let mut pairs = PairVec::new();
    for _ in 0..10 {
        pairs.push(tracker.make_item(), tracker.make_item());
    }
    drop(pairs.pop());
    // The clone makes a new item for each of the 18 items left, and dropping
    // the clone drops those.
    drop(pairs.clone());
    pairs.clear();
    pairs.push(tracker.make_item(), tracker.make_item());
    drop(pairs);
    tracker.assert_all_dropped_exactly_once(tracker.total_made());
    assert_eq!(tracker.total_made(), 40);
}

#[test]
fn clear_drops_every_item_and_empties_the_vec_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let mut pairs = PairVec::new();
    // Dropping the third item of the first slice panics. The fifth item of
    // the second slice is armed too, but it is dropped while that panic
    // unwinds, and an armed bomb does not panic then.
    for i in 0..6 {
        pairs.push(Bomb::new(&tracker, i == 2), Bomb::new(&tracker, i == 4));
    }
    assert!(catch_unwind(AssertUnwindSafe(|| pairs.clear())).is_err());
    assert!(pairs.is_empty());
    pairs.push(Bomb::new(&tracker, false), Bomb::new(&tracker, false));
    assert_eq!(pairs.len(), 1);
    drop(pairs);
    tracker.assert_all_dropped_exactly_once(14);
}

#[test]
fn dropping_the_vec_drops_every_item_when_a_drop_in_the_second_slice_panics() {
    let tracker = DropTracker::new();
    let mut pairs = PairVec::new();
    for i in 0..5 {
        pairs.push(Bomb::new(&tracker, false), Bomb::new(&tracker, i == 1));
    }
    assert!(catch_unwind(AssertUnwindSafe(|| drop(pairs))).is_err());
    tracker.assert_all_dropped_exactly_once(10);
}

#[test]
fn into_iter_yields_from_both_ends_and_drops_the_pairs_it_did_not_yield() {
    let tracker = DropTracker::new();
    let pairs: PairVec<u32, _> = (0..6).map(|i| (i, tracker.make_item())).collect();
    let mut iter = pairs.into_iter();
    assert_eq!(iter.len(), 6);
    assert_eq!(iter.next().map(|(i, _)| i), Some(0));
    assert_eq!(iter.next_back().map(|(i, _)| i), Some(5));
    assert_eq!(iter.len(), 4);
    drop(iter);
    tracker.assert_all_dropped_exactly_once(6);
}

#[test]
fn into_iter_drops_the_rest_when_a_drop_panics() {
    let tracker = DropTracker::new();
    let pairs: PairVec<_, _> = (0..4)
        .map(|i| (Bomb::new(&tracker, i == 2), Bomb::new(&tracker, false)))
        .collect();
    let mut iter = pairs.into_iter();
    drop(iter.next());
    assert!(catch_unwind(AssertUnwindSafe(|| drop(iter))).is_err());
    tracker.assert_all_dropped_exactly_once(8);
}

#[test]
fn clone_and_debug_show_the_same_pairs() {
    let pairs: PairVec<u8, &str> = [(1, "a"), (2, "b")].into_iter().collect();
    let copy = pairs.clone();
    assert_eq!(copy.slices(), pairs.slices());
    assert_eq!(format!("{pairs:?}"), r#"[(1, "a"), (2, "b")]"#);
    assert!(PairVec::<u8, u8>::default().is_empty());
}

#[test]
fn a_pair_vec_and_its_iterator_can_move_between_threads() {
    fn assert_send_and_sync<T: Send + Sync>() {}
    assert_send_and_sync::<PairVec<u32, String>>();
    assert_send_and_sync::<PairVecIntoIter<u32, String>>();
}

#[test]
fn a_pair_vec_agrees_with_two_vecs() {
    let (seeds, steps) = if cfg!(miri) { (2, 200) } else { (16, 2000) };
    for seed in 0..seeds {
        let mut rng = Rng(seed);
        let mut pairs = PairVec::<u16, String>::new();
        let mut firsts = Vec::new();
        let mut seconds = Vec::new();
        for _ in 0..steps {
            match rng.below(10) {
                0..=4 => {
                    let first = rng.next() as u16;
                    pairs.push(first, first.to_string());
                    firsts.push(first);
                    seconds.push(first.to_string());
                }
                5 | 6 => assert_eq!(pairs.pop(), firsts.pop().zip(seconds.pop())),
                7 => {
                    let additional = rng.below(20);
                    pairs.ensure_room(additional).unwrap();
                    assert!(pairs.capacity() >= pairs.len() + additional);
                }
                8 if !firsts.is_empty() => {
                    let i = rng.below(firsts.len());
                    let (first, second) = pairs.slices_mut();
                    first[i] = first[i].wrapping_add(1);
                    second[i].push('x');
                    firsts[i] = firsts[i].wrapping_add(1);
                    seconds[i].push('x');
                }
                _ if rng.chance(10) => {
                    pairs.clear();
                    firsts.clear();
                    seconds.clear();
                }
                _ => {}
            }
            assert_eq!(pairs.first_slice(), firsts.as_slice());
            assert_eq!(pairs.second_slice(), seconds.as_slice());
        }
        let expected: Vec<_> = firsts.into_iter().zip(seconds).collect();
        assert_eq!(pairs.into_iter().collect::<Vec<_>>(), expected);
    }
}
