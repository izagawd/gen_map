#[cfg(feature = "alloc")]
use alloc::collections::TryReserveError;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::fmt;

/// The collection a [`GenMap`](crate::GenMap) or a
/// [`SecondaryMap`](crate::SecondaryMap) keeps its slots in. A
/// [`GenMapConfig`](crate::GenMapConfig) or a
/// [`SecondaryMapConfig`](crate::SecondaryMapConfig) chooses one with its
/// `Storage` type. A dense map also keeps its slots in a storage like this,
/// and a [`SplitPair`](crate::SplitPair) keeps the keys and values of a dense
/// map in two of them.
///
/// A storage whose capacity can grow past what it was created with also
/// implements the [`ReserveStorage`] marker. A storage that implements
/// `IntoIterator` gives a map an owning `into_iter`. The iterator that
/// `into_iter` returns implements `DoubleEndedIterator` when the storage's
/// iterator implements both `DoubleEndedIterator` and `ExactSizeIterator`.
///
/// # Safety
///
/// A `GenMap` reads slots without bounds checks at positions it has already
/// checked against [`len`](Self::len), and it keeps a free list of positions
/// whose slots it expects to find unchanged when it reuses them. A
/// `SecondaryMap` builds each value's key from the position of its slot without
/// checking that the position fits in the key, and it reads a slot it has just
/// pushed without a bounds check. A dense map uses its slot storage the way a
/// `GenMap` does, and a [`SplitPair`](crate::SplitPair) relies on its two
/// storages to keep each key at the same position as its value. So a storage
/// must behave like a `Vec` in the ways listed below.
///
/// - [`as_slice`](Self::as_slice) and [`as_mut_slice`](Self::as_mut_slice)
///   must return exactly the items pushed with [`try_push`](Self::try_push)
///   since the last [`clear`](Self::clear) and not taken out by
///   [`pop`](Self::pop) since, in the order they were pushed, and no others.
///   [`len`](Self::len) and [`is_empty`](Self::is_empty) must agree with them,
///   as the provided methods do.
/// - Apart from `pop` and `clear`, and dropping the storage itself, no method
///   may remove, drop, replace or change an item. That includes the methods of
///   any other trait. An item only changes through the slice `as_mut_slice`
///   returns. Growing may move the items in memory, but must keep them as they
///   are.
/// - `try_push` must either append the item at the end and return `Ok`, or
///   hand the item back in `Err` and leave the storage as it was.
/// - `pop` must take out the last item of the slice and return it, or return
///   `None` and leave the storage as it was if it has no items.
/// - Once [`ensure_room`](Self::ensure_room) has returned `Ok` for `n`
///   items, the next `n` calls of `try_push` must succeed, as long as no
///   other `&mut self` method of this trait runs in between.
/// - `clear` must drop every item and leave the storage empty. It must leave
///   the storage empty even when dropping an item panics.
/// - [`empty`](Self::empty) and [`with_capacity`](Self::with_capacity) must
///   return a storage with no items.
/// - If the storage implements `IntoIterator<Item = Self::Item>`,
///   `into_iter` must yield the same items as `as_slice`, in the same order.
/// - If that iterator also implements `DoubleEndedIterator` and
///   `ExactSizeIterator`, `next_back` must yield the items starting from the
///   last one, and `len` must be the number of items not yet yielded.
pub unsafe trait SliceStorage {
    /// The items the storage holds. A `GenMap`'s storage holds its
    /// [`MapSlot`](crate::MapSlot)s, and a `SecondaryMap`'s holds its
    /// [`SecondaryMapSlot`](crate::SecondaryMapSlot)s.
    type Item;

    /// Why the storage could not make room for another item.
    type Error: fmt::Debug;

    /// Creates a storage with no items.
    fn empty() -> Self;

    /// Creates a storage with no items and room for at least `capacity` of
    /// them. A storage whose type limits how many items it can hold, such as an
    /// `ArrayVec`, can ignore the argument.
    fn with_capacity(capacity: usize) -> Self;

    /// How many items the storage can hold before it has to grow, or in
    /// total if it cannot grow.
    fn capacity(&self) -> usize;

    /// Returns every item as a slice, in the order the items were pushed.
    fn as_slice(&self) -> &[Self::Item];

    /// Returns every item as a mutable slice, in the order the items were
    /// pushed.
    fn as_mut_slice(&mut self) -> &mut [Self::Item];

    /// Makes sure the next `additional` calls of
    /// [`try_push`](Self::try_push) will succeed, growing if the storage can
    /// and has to. A `SecondaryMap` asks for room up to a key's index, so
    /// `additional` can be larger than any storage can hold. The map expects
    /// an error in that case, not a panic.
    ///
    /// # Errors
    ///
    /// Returns the reason the pushes would fail if the storage cannot make
    /// room for all of them.
    fn ensure_room(&mut self, additional: usize) -> Result<(), Self::Error>;

    /// Appends `item`.
    ///
    /// # Errors
    ///
    /// Hands `item` back if the storage cannot make room for it.
    fn try_push(&mut self, item: Self::Item) -> Result<(), Self::Item>;

    /// Takes out the last item and returns it, or returns `None` if there are
    /// no items.
    fn pop(&mut self) -> Option<Self::Item>;

    /// Drops every item, which leaves the storage empty.
    fn clear(&mut self);

    /// The number of items.
    #[inline]
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Returns `true` if there are no items.
    #[inline]
    fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }
}

/// Marks a [`SliceStorage`] whose capacity can grow past what it was created
/// with. [`ensure_room`](SliceStorage::ensure_room) grows such a storage when
/// it needs more room. A map has `with_capacity_and_config`, `reserve` and
/// `try_reserve` only when its slot storage implements this trait. A dense map
/// also needs [`ReservePairStorage`](crate::ReservePairStorage) on its pair
/// storage, which a [`SplitPair`](crate::SplitPair) implements when both of
/// its storages implement this trait.
pub trait ReserveStorage: SliceStorage {}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
// SAFETY: a `Vec` behaves exactly as the trait describes. `Vec::push` itself
// panics on capacity overflow and aborts on allocation failure, so both
// `ensure_room` and `try_push` go through `try_reserve`, which reports the
// two as errors. Once `try_reserve` has made room for `additional` items, the
// next `additional` pushes cannot fail.
unsafe impl<S> SliceStorage for Vec<S> {
    type Item = S;
    type Error = TryReserveError;

    #[inline]
    fn empty() -> Self {
        Vec::new()
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        Vec::with_capacity(capacity)
    }

    #[inline]
    fn capacity(&self) -> usize {
        Vec::capacity(self)
    }

    #[inline]
    fn as_slice(&self) -> &[S] {
        self
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [S] {
        self
    }

    #[inline]
    fn ensure_room(&mut self, additional: usize) -> Result<(), TryReserveError> {
        Vec::try_reserve(self, additional)
    }

    #[inline]
    fn try_push(&mut self, item: S) -> Result<(), S> {
        match Vec::try_reserve(self, 1) {
            Ok(()) => {
                // Room was just made, so this can neither grow nor fail.
                Vec::push(self, item);
                Ok(())
            }
            Err(_) => Err(item),
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<S> {
        Vec::pop(self)
    }

    #[inline]
    fn clear(&mut self) {
        Vec::clear(self)
    }
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<S> ReserveStorage for Vec<S> {}

/// Storage from the `arrayvec` crate that keeps up to `CAP` items inline and
/// never allocates. It needs the `arrayvec` feature.
///
/// The storage cannot grow. When it holds the slots of a
/// [`GenMap`](crate::GenMap), inserting fails with
/// [`FullError::StorageFull`](crate::FullError::StorageFull) once all `CAP`
/// slots exist and none of them are free. If the map's keys run out of
/// indices for new slots first, or at the same time, inserting fails with
/// [`FullError::IndexExhausted`](crate::FullError::IndexExhausted) instead.
/// When it holds the slots of a
/// [`SecondaryMap`](crate::SecondaryMap), inserting under a key whose index
/// is `CAP` or more fails with
/// [`SecondaryInsertError::StorageFull`](crate::SecondaryInsertError::StorageFull),
/// unless the index is the largest value of the index type, which fails with
/// [`SecondaryInsertError::IndexReserved`](crate::SecondaryInsertError::IndexReserved).
///
/// ```
/// use arrayvec::ArrayVec;
/// use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
///
/// /// Maps with this config keep up to sixteen slots inline and never
/// /// allocate.
/// struct Inline;
///
/// impl MapConfig for Inline {
///     type KeyConfig = Split<u8, u8>;
/// }
///
/// impl GenMapConfig for Inline {
///     type Storage<S: GenSlotItem> = ArrayVec<S, 16>;
/// }
///
/// let mut map = GenMap::<u32, Inline>::new_with_config();
/// let key = map.insert(1);
/// assert_eq!(map[key], 1);
/// assert_eq!(map.capacity(), 16);
/// ```
#[cfg(feature = "arrayvec")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrayvec")))]
// SAFETY: an `ArrayVec` keeps its items in order and in place, like a
// `Vec`. Its `try_push` only fails when it is full, and `ensure_room`
// returns `Ok` only when it has room for all `additional` items. Its `clear`
// sets the length to zero before it drops the items, so the storage is empty
// even when a drop panics.
unsafe impl<S, const CAP: usize> SliceStorage for arrayvec::ArrayVec<S, CAP> {
    type Item = S;
    type Error = arrayvec::CapacityError;

    #[inline]
    fn empty() -> Self {
        arrayvec::ArrayVec::new()
    }

    /// An `ArrayVec` always has room for exactly `CAP` items, so the
    /// `capacity` argument is ignored.
    #[inline]
    fn with_capacity(_capacity: usize) -> Self {
        arrayvec::ArrayVec::new_const()
    }

    #[inline]
    fn capacity(&self) -> usize {
        CAP
    }

    #[inline]
    fn as_slice(&self) -> &[S] {
        self
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [S] {
        self
    }

    #[inline]
    fn ensure_room(&mut self, additional: usize) -> Result<(), arrayvec::CapacityError> {
        if additional <= arrayvec::ArrayVec::remaining_capacity(self) {
            Ok(())
        } else {
            Err(arrayvec::CapacityError::new(()))
        }
    }

    #[inline]
    fn try_push(&mut self, item: S) -> Result<(), S> {
        arrayvec::ArrayVec::try_push(self, item).map_err(arrayvec::CapacityError::element)
    }

    #[inline]
    fn pop(&mut self) -> Option<S> {
        arrayvec::ArrayVec::pop(self)
    }

    #[inline]
    fn clear(&mut self) {
        arrayvec::ArrayVec::clear(self);
    }
}

/// Storage from the `smallvec` crate that keeps up to `N` items inline and
/// moves them to the heap once there are more. It needs the `smallvec`
/// feature, which uses the 2.0 beta of `smallvec`.
/// Until smallvec 2.0 is released, a newer smallvec beta or a new release of
/// `gen_map` may break this feature, so it is not covered by semver.
///
/// ```
/// use gen_map::{GenMap, GenMapConfig, GenSlotItem, MapConfig, Split};
/// use smallvec::SmallVec;
///
/// /// Maps with this config keep up to eight slots inline before they
/// /// allocate.
/// struct Small;
///
/// impl MapConfig for Small {
///     type KeyConfig = Split<u32, u32>;
/// }
///
/// impl GenMapConfig for Small {
///     type Storage<S: GenSlotItem> = SmallVec<S, 8>;
/// }
///
/// let mut map = GenMap::<u32, Small>::new_with_config();
/// let keys: Vec<_> = (0..20).map(|i| map.insert(i)).collect();
/// assert_eq!(map[keys[19]], 19);
/// ```
#[cfg(feature = "smallvec")]
#[cfg_attr(docsrs, doc(cfg(feature = "smallvec")))]
// SAFETY: a `SmallVec` behaves like a `Vec` whether its items are inline or
// on the heap. `SmallVec::push` panics or aborts when it cannot grow, so
// both `ensure_room` and `try_push` go through `try_reserve`, which returns
// an error instead. Once `try_reserve` has made room for `additional` items,
// the next `additional` pushes cannot fail.
unsafe impl<S, const N: usize> SliceStorage for smallvec::SmallVec<S, N> {
    type Item = S;
    type Error = smallvec::SmallVecError;

    #[inline]
    fn empty() -> Self {
        smallvec::SmallVec::new()
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        smallvec::SmallVec::with_capacity(capacity)
    }

    #[inline]
    fn capacity(&self) -> usize {
        smallvec::SmallVec::capacity(self)
    }

    #[inline]
    fn as_slice(&self) -> &[S] {
        self
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [S] {
        self
    }

    #[inline]
    fn ensure_room(&mut self, additional: usize) -> Result<(), smallvec::SmallVecError> {
        smallvec::SmallVec::try_reserve(self, additional)
    }

    #[inline]
    fn try_push(&mut self, item: S) -> Result<(), S> {
        match smallvec::SmallVec::try_reserve(self, 1) {
            Ok(()) => {
                // Room was just made, so this can neither grow nor fail.
                smallvec::SmallVec::push(self, item);
                Ok(())
            }
            Err(_) => Err(item),
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<S> {
        smallvec::SmallVec::pop(self)
    }

    #[inline]
    fn clear(&mut self) {
        smallvec::SmallVec::clear(self);
    }
}

#[cfg(feature = "smallvec")]
#[cfg_attr(docsrs, doc(cfg(feature = "smallvec")))]
impl<S, const N: usize> ReserveStorage for smallvec::SmallVec<S, N> {}
