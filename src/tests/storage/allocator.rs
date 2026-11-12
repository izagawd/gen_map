//! These tests cover `SingleVec`, `PairVec` and `Vec` with an allocator
//! other than the global one. The allocators of these tests record every
//! block they hand out, so the tests also check that each block is freed
//! exactly once, with the layout it was allocated with and through an
//! allocator that may free it.

use crate::tests::model::Rng;
use crate::tests::{Cfg, DropItem, DropTracker};
use crate::{
    DenseGenMap, DenseGenMapConfig, DenseSecondaryMap, DenseSecondaryMapConfig, GenMap,
    GenMapConfig, GenSlotItem, KeepLen, MapConfig, NewerWins, PairStorage, PairVec,
    PairVecIntoIter, ReserveError, ReserveStorage, SecondaryMap, SecondaryMapConfig,
    SecondarySlotItem, SingleVec, SingleVecIntoIter, SingleVecRawParts, SliceStorage, Split,
    SplitPair,
};
use core::cell::{Cell, RefCell};
use core::fmt::Debug;
use core::mem::ManuallyDrop;
use core::ptr::{self, NonNull};
use std::alloc::{AllocError, Allocator, Global, Layout, System};
use std::collections::HashMap;
use std::rc::Rc;
use std::string::{String, ToString};
use std::vec::Vec;

/// A `Ledger` is what a [`Tracking`] allocator and its clones record about
/// their blocks.
#[derive(Default)]
struct Ledger {
    /// `live` maps the address of each block that is allocated and not freed
    /// yet to the layout the block was allocated with.
    live: RefCell<HashMap<usize, Layout>>,
    /// `allocations` is the number of blocks allocated so far.
    allocations: Cell<usize>,
    /// `misuses` is the number of times a block was freed although `live` did
    /// not hold it with the layout it was freed with.
    misuses: Cell<usize>,
    /// The allocator refuses to allocate while `refusing` is `true`.
    refusing: Cell<bool>,
}

/// A `Tracking` allocator takes its memory from the global allocator and
/// records every block in a [`Ledger`]. Its clones share the ledger, so each
/// of them can free the blocks of the others.
#[derive(Clone, Default)]
struct Tracking {
    ledger: Rc<Ledger>,
}

impl Tracking {
    /// Returns the number of blocks that are allocated and not freed yet.
    fn live(&self) -> usize {
        self.ledger.live.borrow().len()
    }

    /// Returns the number of blocks allocated so far.
    fn allocations(&self) -> usize {
        self.ledger.allocations.get()
    }

    /// Makes the allocator refuse to allocate if `refusing` is `true`, and
    /// allocate again if it is `false`.
    fn set_refusing(&self, refusing: bool) {
        self.ledger.refusing.set(refusing);
    }

    /// Returns `true` if `other` shares the ledger of this allocator.
    fn shares_a_ledger_with(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.ledger, &other.ledger)
    }

    /// Checks that every block was freed, and that each one was freed with
    /// its own layout through an allocator that shares this ledger.
    fn assert_all_freed(&self) {
        assert_eq!(
            self.ledger.misuses.get(),
            0,
            "a block was freed with the wrong layout or through the wrong allocator"
        );
        assert_eq!(self.live(), 0, "a block was never freed");
    }
}

unsafe impl Allocator for Tracking {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if self.ledger.refusing.get() {
            return Err(AllocError);
        }
        let block = Global.allocate(layout)?;
        let address = block.addr().get();
        self.ledger.live.borrow_mut().insert(address, layout);
        self.ledger.allocations.set(self.allocations() + 1);
        Ok(block)
    }

    /// Frees the block if the ledger holds it with `layout`. Otherwise it
    /// leaves the block alone and counts a misuse.
    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        let recorded = self.ledger.live.borrow_mut().remove(&pointer.addr().get());
        if recorded == Some(layout) {
            unsafe { Global.deallocate(pointer, layout) };
        } else {
            self.ledger.misuses.set(self.ledger.misuses.get() + 1);
        }
    }
}

/// A `Separate` allocator works like a [`Tracking`] allocator, but its clone
/// starts a ledger of its own. So a clone cannot free the blocks of the
/// allocator it was cloned from, and that allocator cannot free the blocks of
/// the clone.
#[derive(Default)]
struct Separate(Tracking);

impl Clone for Separate {
    fn clone(&self) -> Self {
        Self::default()
    }
}

unsafe impl Allocator for Separate {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.0.allocate(layout)
    }

    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        unsafe { self.0.deallocate(pointer, layout) };
    }
}

/// Returns the ids of the items in `storage`, in order.
fn ids<St: SliceStorage<Item = DropItem>>(storage: &St) -> Vec<u32> {
    storage.as_slice().iter().map(|item| item.id).collect()
}

/// Checks a storage that `make` creates against what `SliceStorage` promises
/// about pushing, popping, cloning and clearing, and checks that every item
/// is dropped exactly once.
fn check_storage<St>(make: impl Fn() -> St)
where
    St: ReserveStorage<Item = DropItem> + Clone + IntoIterator<Item = DropItem>,
    St::IntoIter: DoubleEndedIterator,
{
    let tracker = DropTracker::new();
    let mut storage = make();
    assert!(storage.is_empty());
    assert!(storage.ensure_room(6).is_ok());
    for _ in 0..6 {
        unsafe { storage.push_unchecked(tracker.make_item()) };
    }
    assert!(storage.capacity() >= 6);
    assert_eq!(ids(&storage), [0, 1, 2, 3, 4, 5]);
    assert_eq!(storage.pop().map(|item| item.id), Some(5));
    storage.as_mut_slice().swap(0, 1);
    assert_eq!(ids(&storage), [1, 0, 2, 3, 4]);

    // Each `DropItem` clone takes the next free id, so clones made in item
    // order have ids that count up by one.
    let clone = storage.clone();
    assert_eq!(ids(&clone), [6, 7, 8, 9, 10]);
    let mut target = make();
    target.clone_from(&storage);
    assert_eq!(ids(&target), [11, 12, 13, 14, 15]);

    storage.clear();
    assert!(storage.is_empty());
    let backwards: Vec<_> = clone.into_iter().rev().map(|item| item.id).collect();
    assert_eq!(backwards, [10, 9, 8, 7, 6]);
    drop(storage);
    drop(target);
    tracker.assert_all_dropped_exactly_once(16);
}

#[test]
fn vecs_with_an_allocator_keep_the_storage_contract() {
    let allocator = Tracking::default();
    check_storage(|| Vec::new_in(allocator.clone()));
    check_storage(|| SingleVec::<_, u8, _>::new_in(allocator.clone()));
    check_storage(|| SingleVec::<_, usize, _>::with_capacity_in(2, allocator.clone()));
    assert!(allocator.allocations() > 0);
    allocator.assert_all_freed();
}

#[test]
fn a_single_vec_allocates_and_frees_through_its_allocator() {
    let allocator = Tracking::default();
    let mut items = SingleVec::<u32, u16, _>::new_in(allocator.clone());
    assert!(items.allocator().shares_a_ledger_with(&allocator));
    assert_eq!(allocator.allocations(), 0);

    for i in 0..100 {
        items.push(i);
    }
    assert!(items.iter().copied().eq(0..100));
    // Growing frees the buffer it replaces, so only one buffer is left.
    assert!(allocator.allocations() > 1);
    assert_eq!(allocator.live(), 1);

    drop(items);
    allocator.assert_all_freed();
}

#[test]
fn a_single_vec_with_capacity_in_allocates_its_buffer_once() {
    let allocator = Tracking::default();
    let mut items = SingleVec::<u64, u32, _>::with_capacity_in(8, allocator.clone());
    assert_eq!(items.capacity(), 8);
    for i in 0..8 {
        items.push(i);
    }
    assert_eq!(allocator.allocations(), 1);
    drop(items);
    allocator.assert_all_freed();
}

#[test]
fn a_single_vec_of_items_that_take_no_space_never_uses_its_allocator() {
    let allocator = Tracking::default();
    let mut units = SingleVec::<(), u8, _>::with_capacity_in(3, allocator.clone());
    for _ in 0..255 {
        units.push(());
    }
    let mut clone = units.clone();
    clone.clone_from(&units);
    assert_eq!(clone.len(), 255);
    assert_eq!(units.into_iter().count(), 255);
    assert_eq!(allocator.allocations(), 0);
}

#[test]
fn a_single_vec_keeps_its_items_when_its_allocator_refuses() {
    let allocator = Tracking::default();
    let mut items = SingleVec::<u32, u32, _>::with_capacity_in(2, allocator.clone());
    items.push(1);
    items.push(2);

    allocator.set_refusing(true);
    assert!(matches!(
        items.ensure_room(1),
        Err(ReserveError::AllocError(_))
    ));
    assert_eq!(items.try_push(3), Err(3));
    assert_eq!(items[..], [1, 2]);
    assert_eq!(items.capacity(), 2);

    allocator.set_refusing(false);
    assert_eq!(items.try_push(3), Ok(()));
    assert_eq!(items[..], [1, 2, 3]);
    drop(items);
    allocator.assert_all_freed();
}

#[test]
fn a_clone_of_a_single_vec_allocates_from_a_clone_of_its_allocator() {
    let allocator = Tracking::default();
    let mut items = SingleVec::<u32, u32, _>::new_in(allocator.clone());
    items.push(1);
    items.push(2);

    let clone = items.clone();
    assert_eq!(clone[..], [1, 2]);
    assert!(clone.allocator().shares_a_ledger_with(&allocator));
    assert_eq!(allocator.live(), 2);
    drop(items);
    drop(clone);
    allocator.assert_all_freed();
}

#[test]
fn a_single_vec_and_its_clone_never_free_the_buffer_of_the_other() {
    let mut items = SingleVec::<u32, u32, _>::new_in(Separate::default());
    for i in 0..20 {
        items.push(i);
    }
    let mut clone = items.clone();
    // These two allocators share the ledgers of the two vecs, so they still
    // see the ledgers after the vecs are dropped.
    let first = items.allocator().0.clone();
    let second = clone.allocator().0.clone();
    assert!(!first.shares_a_ledger_with(&second));
    assert_eq!((first.live(), second.live()), (1, 1));

    // The buffer of the clone is too small for a hundred items, so
    // `clone_from` replaces it.
    for i in 20..100 {
        items.push(i);
    }
    clone.clone_from(&items);
    assert!(clone.iter().copied().eq(0..100));
    assert_eq!((first.live(), second.live()), (1, 1));

    drop(items);
    drop(clone);
    first.assert_all_freed();
    second.assert_all_freed();
}

#[test]
fn clone_from_allocates_from_the_allocator_of_the_single_vec_it_fills() {
    let source_allocator = Tracking::default();
    let allocator = Tracking::default();
    let mut source = SingleVec::<u32, u32, _>::new_in(source_allocator.clone());
    for i in 0..20 {
        source.push(i);
    }
    let source_allocations = source_allocator.allocations();
    let mut items = SingleVec::<u32, u32, _>::with_capacity_in(2, allocator.clone());
    items.push(100);

    // The buffer is too small, so `clone_from` allocates another one.
    items.clone_from(&source);
    assert!(items.iter().copied().eq(0..20));
    assert!(items.allocator().shares_a_ledger_with(&allocator));
    assert_eq!((allocator.allocations(), allocator.live()), (2, 1));

    // The buffer has room now, so `clone_from` reuses it.
    let pointer = items.as_ptr();
    items.clone_from(&source);
    assert_eq!(items.as_ptr(), pointer);
    assert_eq!(allocator.allocations(), 2);
    assert_eq!(source_allocator.allocations(), source_allocations);

    drop(items);
    allocator.assert_all_freed();
}

#[test]
fn the_owning_iterator_of_a_single_vec_frees_the_buffer_through_its_allocator() {
    let allocator = Tracking::default();
    let tracker = DropTracker::new();
    let mut items = SingleVec::<_, u32, _>::new_in(allocator.clone());
    for _ in 0..5 {
        items.push(tracker.make_item());
    }
    let mut iter = items.into_iter();
    assert_eq!(iter.next().map(|item| item.id), Some(0));
    assert_eq!(iter.next_back().map(|item| item.id), Some(4));
    assert_eq!(allocator.live(), 1);
    drop(iter);
    allocator.assert_all_freed();
    tracker.assert_all_dropped_exactly_once(5);
}

#[test]
fn raw_parts_give_back_the_same_single_vec_with_its_allocator() {
    let allocator = Tracking::default();
    let mut items = SingleVec::<u32, u16, _>::with_capacity_in(8, allocator.clone());
    items.push(1);
    items.push(2);

    let parts = items.into_raw_parts();
    assert_eq!((parts.capacity, parts.len), (8, 2));
    assert!(parts.allocator.shares_a_ledger_with(&allocator));
    assert_eq!(allocator.live(), 1);

    let mut items = unsafe { SingleVec::from_raw_parts(parts) };
    assert_eq!((items.capacity(), items.len()), (8, 2));
    for i in 3..=20 {
        items.push(i);
    }
    assert!(items.iter().copied().eq(1..=20));
    drop(items);
    allocator.assert_all_freed();
}

#[test]
fn a_std_vec_takes_over_the_buffer_and_the_allocator_of_a_single_vec() {
    let allocator = Tracking::default();
    let mut items = SingleVec::<u32, u16, _>::with_capacity_in(8, allocator.clone());
    for i in 0..5 {
        items.push(i);
    }
    let pointer = items.as_ptr();

    let mut vec = Vec::from(items);
    assert_eq!(vec.as_ptr(), pointer);
    assert_eq!((vec.capacity(), vec.len()), (8, 5));
    assert!(vec.allocator().shares_a_ledger_with(&allocator));
    assert_eq!(allocator.allocations(), 1);
    // Growing reallocates the buffer of the `SingleVec`.
    vec.extend(5..100);
    assert!(vec.iter().copied().eq(0..100));
    drop(vec);
    allocator.assert_all_freed();
}

#[test]
fn a_single_vec_takes_over_the_buffer_and_the_allocator_of_a_std_vec() {
    let allocator = Tracking::default();
    let mut vec = Vec::with_capacity_in(3, allocator.clone());
    vec.extend(0..3u64);
    let mut vec = ManuallyDrop::new(vec);
    let parts = SingleVecRawParts {
        pointer: NonNull::new(vec.as_mut_ptr()).unwrap(),
        capacity: vec.capacity(),
        len: vec.len(),
        // The `Vec` is never dropped, so its allocator is only moved out once.
        allocator: unsafe { ptr::read(vec.allocator()) },
    };

    let mut items: SingleVec<_, usize, _> = unsafe { SingleVec::from_raw_parts(parts) };
    // Growing reallocates the buffer of the `Vec`.
    for i in 3..100 {
        items.push(i);
    }
    assert!(items.iter().copied().eq(0..100));
    drop(items);
    allocator.assert_all_freed();
}

#[test]
fn a_single_vec_with_an_allocator_agrees_with_a_vec() {
    let (seeds, steps) = if cfg!(miri) { (2, 200) } else { (8, 1000) };
    for seed in 0..seeds {
        let allocator = Tracking::default();
        let mut rng = Rng(seed);
        let mut items = SingleVec::<String, u16, _>::new_in(allocator.clone());
        let mut copy = SingleVec::<String, u16, _>::new_in(allocator.clone());
        let mut expected = Vec::new();
        for _ in 0..steps {
            match rng.below(10) {
                0..=4 => {
                    let item = rng.next().to_string();
                    items.push(item.clone());
                    expected.push(item);
                }
                5 => assert_eq!(items.pop(), expected.pop()),
                6 => {
                    let additional = rng.below(20);
                    items.ensure_room(additional).unwrap();
                    assert!(items.capacity() >= items.len() + additional);
                }
                7 => {
                    copy.clone_from(&items);
                    assert_eq!(copy[..], expected[..]);
                }
                8 => {
                    copy = items.clone();
                    assert_eq!(copy[..], expected[..]);
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
        drop(copy);
        allocator.assert_all_freed();
    }
}

#[test]
fn a_pair_vec_allocates_and_frees_through_its_allocator() {
    let allocator = Tracking::default();
    let mut pairs = PairVec::<u8, u64, u16, _>::new_in(allocator.clone());
    assert!(pairs.allocator().shares_a_ledger_with(&allocator));
    assert_eq!(allocator.allocations(), 0);

    for i in 0..100 {
        pairs.push(i, u64::from(i) * 2);
    }
    assert!(pairs.first_slice().iter().copied().eq(0..100));
    assert!(pairs
        .second_slice()
        .iter()
        .copied()
        .eq((0..100).map(|i| i * 2)));
    // Growing frees the buffer it replaces, so only one buffer is left.
    assert!(allocator.allocations() > 1);
    assert_eq!(allocator.live(), 1);

    assert_eq!(pairs.pop(), Some((99, 198)));
    drop(pairs);
    allocator.assert_all_freed();
}

#[test]
fn a_pair_vec_with_capacity_in_allocates_its_buffer_once() {
    let allocator = Tracking::default();
    let mut pairs = PairVec::<u64, u8, u32, _>::with_capacity_in(8, allocator.clone());
    assert_eq!(pairs.capacity(), 8);
    for i in 0..8 {
        pairs.push(u64::from(i), i);
    }
    assert_eq!(allocator.allocations(), 1);
    drop(pairs);
    allocator.assert_all_freed();
}

#[test]
fn a_pair_vec_keeps_its_pairs_when_its_allocator_refuses() {
    let allocator = Tracking::default();
    let mut pairs = PairVec::<u32, u8, u32, _>::with_capacity_in(1, allocator.clone());
    pairs.push(1, 2);

    allocator.set_refusing(true);
    assert!(matches!(
        pairs.ensure_room(1),
        Err(ReserveError::AllocError(_))
    ));
    assert_eq!(pairs.try_push(3, 4), Err((3, 4)));
    assert_eq!(pairs.slices(), (&[1][..], &[2][..]));
    assert_eq!(pairs.capacity(), 1);

    allocator.set_refusing(false);
    assert_eq!(pairs.try_push(3, 4), Ok(()));
    assert_eq!(pairs.slices(), (&[1, 3][..], &[2, 4][..]));
    drop(pairs);
    allocator.assert_all_freed();
}

#[test]
fn a_clone_of_a_pair_vec_allocates_from_a_clone_of_its_allocator() {
    let allocator = Tracking::default();
    let mut pairs = PairVec::<u32, char, u32, _>::new_in(allocator.clone());
    pairs.push(1, 'a');
    pairs.push(2, 'b');

    let clone = pairs.clone();
    assert_eq!(clone.slices(), (&[1, 2][..], &['a', 'b'][..]));
    assert!(clone.allocator().shares_a_ledger_with(&allocator));
    assert_eq!(allocator.live(), 2);
    drop(pairs);
    drop(clone);
    allocator.assert_all_freed();
}

#[test]
fn a_pair_vec_and_its_clone_never_free_the_buffer_of_the_other() {
    let mut pairs = PairVec::<u32, u8, u32, _>::new_in(Separate::default());
    for i in 0..20 {
        pairs.push(u32::from(i), i);
    }
    let mut clone = pairs.clone();
    // These two allocators share the ledgers of the two vecs, so they still
    // see the ledgers after the vecs are dropped.
    let first = pairs.allocator().0.clone();
    let second = clone.allocator().0.clone();
    assert!(!first.shares_a_ledger_with(&second));
    assert_eq!((first.live(), second.live()), (1, 1));

    // The buffer of the clone is too small for a hundred pairs, so
    // `clone_from` replaces it.
    for i in 20..100 {
        pairs.push(u32::from(i), i);
    }
    clone.clone_from(&pairs);
    assert!(clone.first_slice().iter().copied().eq(0..100));
    assert!(clone.second_slice().iter().copied().eq(0..100));
    assert_eq!((first.live(), second.live()), (1, 1));

    drop(pairs);
    drop(clone);
    first.assert_all_freed();
    second.assert_all_freed();
}

#[test]
fn clone_from_allocates_from_the_allocator_of_the_pair_vec_it_fills() {
    let source_allocator = Tracking::default();
    let allocator = Tracking::default();
    let mut source = PairVec::<u32, u8, u32, _>::new_in(source_allocator.clone());
    for i in 0..20 {
        source.push(u32::from(i), i);
    }
    let source_allocations = source_allocator.allocations();
    let mut pairs = PairVec::<u32, u8, u32, _>::with_capacity_in(2, allocator.clone());
    pairs.push(100, 100);

    // The buffer is too small, so `clone_from` allocates another one.
    pairs.clone_from(&source);
    assert!(pairs.first_slice().iter().copied().eq(0..20));
    assert!(pairs.second_slice().iter().copied().eq(0..20));
    assert!(pairs.allocator().shares_a_ledger_with(&allocator));
    assert_eq!((allocator.allocations(), allocator.live()), (2, 1));

    // The buffer has room now, so `clone_from` reuses it.
    let pointer = pairs.first_slice().as_ptr();
    pairs.clone_from(&source);
    assert_eq!(pairs.first_slice().as_ptr(), pointer);
    assert_eq!(allocator.allocations(), 2);
    assert_eq!(source_allocator.allocations(), source_allocations);

    drop(pairs);
    allocator.assert_all_freed();
}

#[test]
fn the_owning_iterator_of_a_pair_vec_frees_the_buffer_through_its_allocator() {
    let allocator = Tracking::default();
    let tracker = DropTracker::new();
    let mut pairs = PairVec::<_, _, u32, _>::new_in(allocator.clone());
    for i in 0..5 {
        pairs.push(i, tracker.make_item());
    }
    let mut iter = pairs.into_iter();
    assert_eq!(iter.next().map(|(first, _)| first), Some(0));
    assert_eq!(iter.next_back().map(|(first, _)| first), Some(4));
    assert_eq!(allocator.live(), 1);
    drop(iter);
    allocator.assert_all_freed();
    tracker.assert_all_dropped_exactly_once(5);
}

#[test]
fn raw_parts_give_back_the_same_pair_vec_with_its_allocator() {
    let allocator = Tracking::default();
    let mut pairs = PairVec::<u32, char, u16, _>::with_capacity_in(8, allocator.clone());
    pairs.push(1, 'a');
    pairs.push(2, 'b');

    let parts = pairs.into_raw_parts();
    assert_eq!((parts.capacity, parts.len), (8, 2));
    assert!(parts.allocator.shares_a_ledger_with(&allocator));
    assert_eq!(allocator.live(), 1);

    let mut pairs = unsafe { PairVec::from_raw_parts(parts) };
    assert_eq!((pairs.capacity(), pairs.len()), (8, 2));
    for i in 3..=20 {
        pairs.push(i, 'c');
    }
    assert!(pairs.first_slice().iter().copied().eq(1..=20));
    drop(pairs);
    allocator.assert_all_freed();
}

#[test]
fn a_pair_vec_frees_its_buffers_whatever_the_sizes_and_alignments_of_its_items() {
    /// `Aligned` takes no space but needs a large alignment.
    #[derive(Clone, Copy, PartialEq, Debug)]
    #[repr(align(64))]
    struct Aligned;

    fn check<A, B>(make: impl Fn(u8) -> (A, B))
    where
        A: Copy + PartialEq + Debug,
        B: Copy + PartialEq + Debug,
    {
        let allocator = Tracking::default();
        let mut pairs = PairVec::<A, B, u16, _>::new_in(allocator.clone());
        let mut firsts = Vec::new();
        let mut seconds = Vec::new();
        for i in 0..100 {
            let (first, second) = make(i);
            pairs.push(first, second);
            firsts.push(first);
            seconds.push(second);
        }
        pairs.ensure_room(1000).unwrap();
        assert_eq!(pairs.slices(), (firsts.as_slice(), seconds.as_slice()));

        // The buffer of `copy` is too small for a hundred pairs, so
        // `clone_from` replaces it.
        let mut copy = PairVec::<A, B, u16, _>::with_capacity_in(1, allocator.clone());
        copy.clone_from(&pairs);
        assert_eq!(copy.slices(), (firsts.as_slice(), seconds.as_slice()));
        let clone = pairs.clone();
        assert_eq!(clone.slices(), (firsts.as_slice(), seconds.as_slice()));
        assert!(pairs.into_iter().eq(firsts.into_iter().zip(seconds)));

        drop(copy);
        drop(clone);
        allocator.assert_all_freed();
    }

    check(|i| (i, u64::from(i)));
    check(|i| (u64::from(i), i));
    check(|i| ([i; 3], u16::from(i)));
    check(|i| (u16::from(i), [i; 3]));
    check(|i| (u128::from(i), ()));
    check(|i| ((), u128::from(i)));
    check(|i| (i, Aligned));
    check(|i| (Aligned, i));
    check(|_| ((), ()));
}

#[test]
fn a_vec_with_an_allocator_can_be_created_in_a_const() {
    const ITEMS: SingleVec<u32, u8, System> = SingleVec::new_in(System);
    const PAIRS: PairVec<u32, char, u8, System> = PairVec::new_in(System);

    let mut items = ITEMS;
    items.push(1);
    assert_eq!(items[..], [1]);
    let mut pairs = PAIRS;
    pairs.push(1, 'a');
    assert_eq!(pairs.slices(), (&[1][..], &['a'][..]));
}

#[test]
fn the_vecs_and_their_iterators_can_move_between_threads_when_their_allocator_can() {
    fn assert_send_and_sync<T: Send + Sync>() {}
    assert_send_and_sync::<SingleVec<String, u32, System>>();
    assert_send_and_sync::<SingleVecIntoIter<String, u32, System>>();
    assert_send_and_sync::<PairVec<u32, String, u32, System>>();
    assert_send_and_sync::<PairVecIntoIter<u32, String, u32, System>>();
}

/// Maps with this config keep their slots in a `SingleVec` and their keys and
/// values in a `PairVec`. Both allocate from a [`Tracking`] allocator.
struct InOwnVecs;

impl MapConfig for InOwnVecs {
    type KeyConfig = Split<u32, u32>;
}

impl GenMapConfig for InOwnVecs {
    type Len = KeepLen;
    type Storage<S: GenSlotItem> = SingleVec<S, u32, Tracking>;
}

impl SecondaryMapConfig for InOwnVecs {
    type ReplaceStrategy = NewerWins;
    type Len = KeepLen;
    type Storage<S: SecondarySlotItem> = SingleVec<S, u32, Tracking>;
}

impl DenseGenMapConfig for InOwnVecs {
    type SlotStorage<S: GenSlotItem> = SingleVec<S, u32, Tracking>;
    type PairStorage<K, V> = PairVec<K, V, u32, Tracking>;
}

impl DenseSecondaryMapConfig for InOwnVecs {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = SingleVec<S, u32, Tracking>;
    type PairStorage<K, V> = PairVec<K, V, u32, Tracking>;
}

/// Maps with this config keep their slots, keys and values in `Vec`s that
/// allocate from a [`Tracking`] allocator.
struct InStdVecs;

impl MapConfig for InStdVecs {
    type KeyConfig = Split<u32, u32>;
}

impl GenMapConfig for InStdVecs {
    type Len = KeepLen;
    type Storage<S: GenSlotItem> = Vec<S, Tracking>;
}

impl SecondaryMapConfig for InStdVecs {
    type ReplaceStrategy = NewerWins;
    type Len = KeepLen;
    type Storage<S: SecondarySlotItem> = Vec<S, Tracking>;
}

impl DenseGenMapConfig for InStdVecs {
    type SlotStorage<S: GenSlotItem> = Vec<S, Tracking>;
    type PairStorage<K, V> = SplitPair<Vec<K, Tracking>, Vec<V, Tracking>>;
}

impl DenseSecondaryMapConfig for InStdVecs {
    type ReplaceStrategy = NewerWins;
    type SlotStorage<S: GenSlotItem> = Vec<S, Tracking>;
    type PairStorage<K, V> = SplitPair<Vec<K, Tracking>, Vec<V, Tracking>>;
}

/// Inserts fifty values into `map` and removes the first half of them. Then
/// it clones the map and checks that the map and its clone hold the other
/// half.
fn check_gen_map<C>(mut map: GenMap<u32, C>)
where
    C: GenMapConfig<Len = KeepLen>,
    GenMap<u32, C>: Clone,
{
    let keys: Vec<_> = (0..50).map(|value| map.insert(value)).collect();
    for key in &keys[..25] {
        assert!(map.remove(*key).is_some());
    }
    let mut clone = map.clone();
    clone.clone_from(&map);
    for (value, key) in (0..50).zip(&keys) {
        let expected = (value >= 25).then_some(value);
        assert_eq!(map.get(*key).copied(), expected);
        assert_eq!(clone.get(*key).copied(), expected);
    }
    assert_eq!(map.len(), 25);
}

#[test]
fn a_gen_map_keeps_its_slots_in_a_vec_with_an_allocator() {
    let allocator = Tracking::default();
    let map = GenMap::<u32, InOwnVecs>::with_storage(SingleVec::new_in(allocator.clone()));
    check_gen_map(map.unwrap());
    let map = GenMap::<u32, InStdVecs>::with_storage(Vec::new_in(allocator.clone()));
    check_gen_map(map.unwrap());
    assert!(allocator.allocations() > 0);
    allocator.assert_all_freed();
}

/// Inserts a value into `map` under every second one of fifty keys. Then it
/// clones the map and checks that the map and its clone hold those values.
fn check_secondary_map<C>(mut map: SecondaryMap<u32, C>)
where
    C: SecondaryMapConfig<Len = KeepLen> + MapConfig<KeyConfig = Split<u32, u32>>,
    SecondaryMap<u32, C>: Clone,
{
    let mut key_map = GenMap::<(), Cfg<u32, u32>>::new_with_config();
    let keys: Vec<_> = (0..50u32).map(|_| key_map.insert(())).collect();
    for (value, key) in (0..50).zip(&keys).step_by(2) {
        map.insert(*key, value);
    }
    let mut clone = map.clone();
    clone.clone_from(&map);
    for (value, key) in (0..50u32).zip(&keys) {
        let expected = value.is_multiple_of(2).then_some(value);
        assert_eq!(map.get(*key).copied(), expected);
        assert_eq!(clone.get(*key).copied(), expected);
    }
    assert_eq!(map.len(), 25);
}

#[test]
fn a_secondary_map_keeps_its_slots_in_a_vec_with_an_allocator() {
    let allocator = Tracking::default();
    let map = SecondaryMap::<u32, InOwnVecs>::with_storage(SingleVec::new_in(allocator.clone()));
    check_secondary_map(map.unwrap());
    let map = SecondaryMap::<u32, InStdVecs>::with_storage(Vec::new_in(allocator.clone()));
    check_secondary_map(map.unwrap());
    assert!(allocator.allocations() > 0);
    allocator.assert_all_freed();
}

/// Inserts fifty values into `map` and removes the first half of them. Then
/// it clones the map and checks that the map and its clone hold the other
/// half.
fn check_dense_gen_map<C>(mut map: DenseGenMap<u32, C>)
where
    C: DenseGenMapConfig,
    DenseGenMap<u32, C>: Clone,
{
    let keys: Vec<_> = (0..50).map(|value| map.insert(value)).collect();
    for key in &keys[..25] {
        assert!(map.remove(*key).is_some());
    }
    let mut clone = map.clone();
    clone.clone_from(&map);
    for (value, key) in (0..50).zip(&keys) {
        let expected = (value >= 25).then_some(value);
        assert_eq!(map.get(*key).copied(), expected);
        assert_eq!(clone.get(*key).copied(), expected);
    }
    assert_eq!(map.len(), 25);
}

#[test]
fn a_dense_gen_map_keeps_its_storages_in_vecs_with_an_allocator() {
    let allocator = Tracking::default();
    let map = DenseGenMap::<u32, InOwnVecs>::with_storages(
        SingleVec::new_in(allocator.clone()),
        PairVec::new_in(allocator.clone()),
    );
    check_dense_gen_map(map.unwrap());

    let pairs = SplitPair::from_parts(
        Vec::new_in(allocator.clone()),
        Vec::new_in(allocator.clone()),
    );
    let map = DenseGenMap::<u32, InStdVecs>::with_storages(
        Vec::new_in(allocator.clone()),
        pairs.unwrap(),
    );
    check_dense_gen_map(map.unwrap());
    assert!(allocator.allocations() > 0);
    allocator.assert_all_freed();
}

/// Inserts a value into `map` under every second one of fifty keys. Then it
/// clones the map and checks that the map and its clone hold those values.
fn check_dense_secondary_map<C>(mut map: DenseSecondaryMap<u32, C>)
where
    C: DenseSecondaryMapConfig + MapConfig<KeyConfig = Split<u32, u32>>,
    DenseSecondaryMap<u32, C>: Clone,
{
    let mut key_map = GenMap::<(), Cfg<u32, u32>>::new_with_config();
    let keys: Vec<_> = (0..50u32).map(|_| key_map.insert(())).collect();
    for (value, key) in (0..50).zip(&keys).step_by(2) {
        map.insert(*key, value);
    }
    let mut clone = map.clone();
    clone.clone_from(&map);
    for (value, key) in (0..50u32).zip(&keys) {
        let expected = value.is_multiple_of(2).then_some(value);
        assert_eq!(map.get(*key).copied(), expected);
        assert_eq!(clone.get(*key).copied(), expected);
    }
    assert_eq!(map.len(), 25);
}

#[test]
fn a_dense_secondary_map_keeps_its_storages_in_vecs_with_an_allocator() {
    let allocator = Tracking::default();
    let map = DenseSecondaryMap::<u32, InOwnVecs>::with_storages(
        SingleVec::new_in(allocator.clone()),
        PairVec::new_in(allocator.clone()),
    );
    check_dense_secondary_map(map.unwrap());

    let pairs = SplitPair::from_parts(
        Vec::new_in(allocator.clone()),
        Vec::new_in(allocator.clone()),
    );
    let map = DenseSecondaryMap::<u32, InStdVecs>::with_storages(
        Vec::new_in(allocator.clone()),
        pairs.unwrap(),
    );
    check_dense_secondary_map(map.unwrap());
    assert!(allocator.allocations() > 0);
    allocator.assert_all_freed();
}
