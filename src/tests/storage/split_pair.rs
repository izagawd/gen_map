//! Tests for `SplitPair`, which keeps each slice of a pair storage in a
//! slice storage of its own.

use crate::tests::{Bomb, DropTracker};
use crate::{PairStorage, SplitPair, SplitPairError};
use std::format;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec;
use std::vec::Vec;

/// Makes room for a pair with `ensure_room` and pushes it, and panics if the
/// storage cannot make room.
fn push_pair<P: PairStorage>(pairs: &mut P, first: P::First, second: P::Second) {
    pairs
        .ensure_room(1)
        .expect("the pair storage has room for one more pair");
    // SAFETY: `ensure_room` just returned `Ok` for this push.
    unsafe { pairs.push_unchecked(first, second) };
}

#[test]
fn from_parts_needs_two_storages_of_the_same_length() {
    assert!(SplitPair::from_parts(vec![1, 2], vec!["a"]).is_none());
    let pairs = SplitPair::from_parts(vec![1, 2], vec!["a", "b"]).unwrap();
    assert_eq!(pairs.slices(), (&[1, 2][..], &["a", "b"][..]));
    assert_eq!(pairs.into_parts(), (vec![1, 2], vec!["a", "b"]));
}

#[test]
fn pushes_and_pops_move_both_slices_together() {
    let mut pairs = SplitPair::<Vec<u32>, Vec<char>>::empty();
    push_pair(&mut pairs, 1, 'a');
    push_pair(&mut pairs, 2, 'b');
    pairs.first_slice_mut()[0] = 10;
    pairs.second_slice_mut()[1] = 'z';
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs.pop(), Some((2, 'z')));
    assert_eq!(pairs.pop(), Some((10, 'a')));
    assert_eq!(pairs.pop(), None);
    assert!(pairs.is_empty());
}

#[test]
fn clear_empties_both_slices_when_a_drop_in_the_first_slice_panics() {
    let tracker = DropTracker::new();
    let mut pairs = SplitPair::<Vec<Bomb>, Vec<Bomb>>::with_capacity(4);
    for i in 0..4 {
        push_pair(
            &mut pairs,
            Bomb::new(&tracker, i == 1),
            Bomb::new(&tracker, false),
        );
    }
    assert!(catch_unwind(AssertUnwindSafe(|| pairs.clear())).is_err());
    assert!(pairs.first_slice().is_empty());
    assert!(pairs.second_slice().is_empty());
    drop(pairs);
    tracker.assert_all_dropped_exactly_once(8);
}

#[test]
fn into_iter_pairs_the_items_of_both_storages() {
    let pairs = SplitPair::from_parts(vec![1, 2, 3], vec!['a', 'b', 'c']).unwrap();
    let mut iter = pairs.into_iter();
    assert_eq!(iter.len(), 3);
    assert_eq!(iter.next_back(), Some((3, 'c')));
    assert_eq!(iter.next(), Some((1, 'a')));
    assert_eq!(iter.len(), 1);
    assert_eq!(iter.collect::<Vec<_>>(), [(2, 'b')]);
}

#[test]
fn the_error_says_which_storage_is_full() {
    let errors: [SplitPairError<&str, &str>; 2] =
        [SplitPairError::First("a"), SplitPairError::Second("b")];
    let messages: Vec<_> = errors.iter().map(|error| format!("{error}")).collect();
    assert_eq!(
        messages,
        [
            "the storage of the first slice is full: a",
            "the storage of the second slice is full: b",
        ]
    );
}

#[cfg(feature = "arrayvec")]
mod capped {
    use super::*;
    use arrayvec::ArrayVec;

    #[test]
    fn a_full_second_storage_refuses_room_and_changes_nothing() {
        // The first storage has room for four items, but the second only has
        // room for two.
        let mut pairs = SplitPair::<ArrayVec<u8, 4>, ArrayVec<u8, 2>>::empty();
        assert_eq!(pairs.capacity(), 2);
        push_pair(&mut pairs, 1, 10);
        push_pair(&mut pairs, 2, 20);
        assert!(matches!(
            pairs.ensure_room(1),
            Err(SplitPairError::Second(_))
        ));
        assert_eq!(pairs.slices(), (&[1, 2][..], &[10, 20][..]));
    }

    #[test]
    fn a_full_first_storage_refuses_room_and_changes_nothing() {
        let mut pairs = SplitPair::<ArrayVec<u8, 2>, ArrayVec<u8, 4>>::empty();
        push_pair(&mut pairs, 1, 10);
        push_pair(&mut pairs, 2, 20);
        assert!(matches!(
            pairs.ensure_room(1),
            Err(SplitPairError::First(_))
        ));
        assert_eq!(pairs.slices(), (&[1, 2][..], &[10, 20][..]));
    }
}
