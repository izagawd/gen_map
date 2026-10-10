//! Tests for `PairVec` on its own. The dense maps use it as the pair storage
//! of the default config, so the dense map tests cover it as well.

use crate::tests::model::Rng;
use crate::tests::{Bomb, DropItem, DropTracker};
use crate::{PairStorage, PairVec, PairVecIntoIter, PairVecRawParts, ReserveError};
use core::alloc::Layout;
use core::fmt::Debug;
use core::mem::size_of;
use core::ptr::NonNull;
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::string::{String, ToString};
use std::vec::Vec;

#[test]
fn pushes_and_pops_keep_the_two_slices_in_step() {
    let mut pairs = PairVec::<_, _>::new();
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
fn growing_moves_every_pair_into_the_new_buffer() {
    let mut pairs = PairVec::<_, _>::with_capacity(3);
    let capacity = pairs.capacity();
    assert!(capacity >= 3);
    // Pushing as many pairs as there is room for does not grow the buffer.
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
fn growing_keeps_both_slices_whatever_the_sizes_and_alignments_of_their_items() {
    /// `Aligned` takes no space but needs a large alignment.
    #[derive(Clone, Copy, PartialEq, Debug)]
    #[repr(align(64))]
    struct Aligned;

    fn check<A, B>(make: impl Fn(u8) -> (A, B))
    where
        A: Copy + PartialEq + Debug,
        B: Copy + PartialEq + Debug,
    {
        let mut pairs = PairVec::<A, B>::new();
        let mut firsts = Vec::new();
        let mut seconds = Vec::new();
        for i in 0..100 {
            let (first, second) = make(i);
            pairs.push(first, second);
            firsts.push(first);
            seconds.push(second);
            assert_eq!(pairs.slices(), (firsts.as_slice(), seconds.as_slice()));
        }
        // Making room for many more pairs moves the slice at the end of the
        // buffer far from where it was.
        pairs.ensure_room(1000).unwrap();
        assert_eq!(pairs.slices(), (firsts.as_slice(), seconds.as_slice()));
    }

    check(|i| (i, u64::from(i)));
    check(|i| (u64::from(i), i));
    check(|i| ([i; 3], u16::from(i)));
    check(|i| (u16::from(i), [i; 3]));
    check(|i| (u128::from(i), ()));
    check(|i| ((), u128::from(i)));
    check(|i| (i, Aligned));
    check(|i| (Aligned, i));
}

#[test]
fn the_slice_whose_items_take_more_space_starts_the_buffer() {
    let mut pairs = PairVec::<u8, u64>::with_capacity(4);
    let (firsts, seconds) = pairs.slices();
    assert_eq!(firsts.as_ptr() as usize, seconds.as_ptr() as usize + 4 * 8);
    let flipped = PairVec::<u64, u8>::with_capacity(4);
    let (firsts, seconds) = flipped.slices();
    assert_eq!(seconds.as_ptr() as usize, firsts.as_ptr() as usize + 4 * 8);

    // The `u64`s stay at the start after the buffer grows.
    for i in 0..100 {
        pairs.push(i, u64::from(i));
    }
    let capacity = pairs.capacity();
    let (firsts, seconds) = pairs.slices();
    assert_eq!(
        firsts.as_ptr() as usize,
        seconds.as_ptr() as usize + capacity * 8
    );
}

#[test]
fn the_first_slice_starts_the_buffer_when_the_items_take_the_same_space() {
    let pairs = PairVec::<u32, char>::with_capacity(4);
    let (firsts, seconds) = pairs.slices();
    assert_eq!(seconds.as_ptr() as usize, firsts.as_ptr() as usize + 4 * 4);
}

#[test]
fn a_slice_whose_items_take_no_space_starts_the_buffer() {
    let pairs = PairVec::<u64, ()>::with_capacity(4);
    let (firsts, seconds) = pairs.slices();
    assert_eq!(firsts.as_ptr() as usize, seconds.as_ptr() as usize);
}

#[test]
fn try_push_appends_a_pair_like_push() {
    let mut pairs = PairVec::<_, _>::new();
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
        Err(ReserveError::CapacityOverflow)
    );
    // A buffer with room for this many `u64`s would take more than
    // `isize::MAX` bytes.
    assert_eq!(
        pairs.ensure_room(usize::MAX / 8),
        Err(ReserveError::CapacityOverflow)
    );
    assert_eq!(pairs.slices(), (&[1][..], &[2][..]));
}

#[test]
fn the_error_says_why_there_is_no_room() {
    assert_eq!(
        format!("{}", ReserveError::CapacityOverflow),
        "the vec cannot hold that many items"
    );
    assert_eq!(
        format!("{}", ReserveError::AllocError(Layout::new::<u64>())),
        "the allocator could not allocate 8 bytes for a buffer"
    );
}

#[test]
fn the_length_type_caps_how_many_pairs_fit() {
    let mut pairs = PairVec::<u32, u32, u8>::new();
    for i in 0..255 {
        pairs.push(i, i * 2);
    }
    assert_eq!(pairs.capacity(), 255);
    assert_eq!(pairs.try_push(255, 510), Err((255, 510)));
    assert_eq!(pairs.ensure_room(1), Err(ReserveError::CapacityOverflow));
    assert_eq!(pairs.len(), 255);
    assert_eq!(pairs.pop(), Some((254, 508)));
    assert!(pairs.try_push(254, 508).is_ok());
    assert!(pairs.into_iter().eq((0..255).map(|i| (i, i * 2))));
}

#[test]
fn a_capacity_the_length_type_cannot_count_panics() {
    assert_eq!(PairVec::<u8, u8, u8>::with_capacity(255).capacity(), 255);
    assert!(catch_unwind(|| PairVec::<u8, u8, u8>::with_capacity(256)).is_err());
    assert!(catch_unwind(|| PairVec::<(), (), u8>::with_capacity(256)).is_err());
}

#[test]
fn a_length_type_counts_pairs_whose_items_take_no_space() {
    let mut units = PairVec::<(), (), u8>::new();
    assert_eq!(units.capacity(), 255);
    for _ in 0..255 {
        units.push((), ());
    }
    assert_eq!(units.try_push((), ()), Err(((), ())));
    assert_eq!(units.into_iter().count(), 255);
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
        Err(ReserveError::CapacityOverflow)
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
    let mut pairs = PairVec::<_, _>::new();
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
    let mut pairs = PairVec::<_, _>::new();
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
    let mut pairs = PairVec::<_, _>::new();
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

#[test]
fn raw_parts_give_back_the_same_vec() {
    let tracker = DropTracker::new();
    let mut pairs = PairVec::<_, _, u16>::with_capacity(8);
    for i in 0..5u32 {
        pairs.push(i, tracker.make_item());
    }
    let parts = pairs.into_raw_parts();
    assert_eq!((parts.capacity, parts.len), (8, 5));
    tracker.assert_none_dropped();
    let pairs = unsafe { PairVec::from_raw_parts(parts) };
    assert_eq!((pairs.capacity(), pairs.len()), (8, 5));
    assert!(pairs.first_slice().iter().copied().eq(0..5));
    drop(pairs);
    tracker.assert_all_dropped_exactly_once(5);
}

#[test]
fn raw_parts_of_pairs_that_take_no_space_keep_their_count() {
    let mut pairs = PairVec::<(), (), u8>::new();
    for _ in 0..3 {
        pairs.push((), ());
    }
    let mut parts = pairs.into_raw_parts();
    assert_eq!((parts.capacity, parts.len), (255, 3));
    // Items that take no space need no buffer, so any capacity of at least
    // `len` follows the rules, and the vec's capacity is still the largest
    // `u8`.
    parts.capacity = 3;
    let pairs = unsafe { PairVec::from_raw_parts(parts) };
    assert_eq!((pairs.capacity(), pairs.len()), (255, 3));
}

#[test]
fn a_vec_can_take_over_a_buffer_allocated_by_hand() {
    let tracker = DropTracker::new();
    assert!(size_of::<DropItem>() > size_of::<u32>());
    let (layout, offset) = Layout::array::<DropItem>(4)
        .unwrap()
        .extend(Layout::array::<u32>(4).unwrap())
        .unwrap();
    let buffer = NonNull::new(unsafe { std::alloc::alloc(layout) }).unwrap();
    let second = buffer.cast::<DropItem>();
    let first = unsafe { buffer.add(offset) }.cast::<u32>();
    for i in 0..3u32 {
        unsafe {
            first.add(i as usize).write(i);
            second.add(i as usize).write(tracker.make_item());
        }
    }
    let parts = PairVecRawParts {
        first,
        second,
        capacity: 4usize,
        len: 3,
    };
    let mut pairs: PairVec<_, _> = unsafe { PairVec::from_raw_parts(parts) };
    // Growing reallocates the buffer that was allocated by hand.
    for i in 3..10 {
        pairs.push(i, tracker.make_item());
    }
    assert!(pairs.first_slice().iter().copied().eq(0..10));
    drop(pairs);
    tracker.assert_all_dropped_exactly_once(10);
}
