#[cfg(feature = "alloc")]
mod buffer;
pub(crate) mod pair;
#[cfg(feature = "alloc")]
pub(crate) mod pair_vec;
#[cfg(feature = "alloc")]
pub(crate) mod single_vec;

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
/// implements the [`ReserveStorage`] marker. A storage that can be created
/// with room for a number of items chosen at runtime implements
/// [`WithCapacity`]. A storage that implements `IntoIterator` gives a map an
/// owning `into_iter`. The iterator that `into_iter` returns implements
/// `DoubleEndedIterator` when the storage's iterator implements both
/// `DoubleEndedIterator` and `ExactSizeIterator`. A map implements `Clone`
/// when its storages do.
///
/// # Safety
///
/// A `GenMap` reads slots without bounds checks at positions it has already
/// checked against [`len`](Self::len), and it keeps a free list of positions
/// whose slots it expects to find unchanged when it reuses them. A
/// `SecondaryMap` builds each value's key from the position of its slot without
/// checking that the position fits in the key, and it reads a slot it has just
/// pushed without a bounds check. A `DenseGenMap` uses its slot storage the
/// way a `GenMap` does, and a `DenseSecondaryMap` uses its slot storage the
/// way a `SecondaryMap` does. A [`SplitPair`](crate::SplitPair) relies on its
/// two storages to keep each key at the same position as its value. So a
/// storage must behave like a `Vec` in the ways listed below.
///
/// - [`as_slice`](Self::as_slice) and [`as_mut_slice`](Self::as_mut_slice)
///   must return exactly the items pushed with
///   [`push_unchecked`](Self::push_unchecked) since the last
///   [`clear`](Self::clear) and not taken out by [`pop`](Self::pop) since,
///   in the order they were pushed, and no others. [`len`](Self::len) and
///   [`is_empty`](Self::is_empty) must agree with them, as the provided
///   methods do.
/// - Apart from `pop` and `clear`, and dropping the storage itself, no method/function
///   implemented in this trait may remove, drop, replace or change an item.
///   An item only mutates through the slice methods. Growing
///   may move the items in memory, but must keep them in the same order and indices.
/// - `pop` must take out the last item of the slice and return it, or return
///   `None` and leave the storage as it was if it has no items.
/// - Once [`ensure_room`](Self::ensure_room) has returned `Ok` for `n`
///   items, the storage must have room for the next `n` calls of
///   `push_unchecked`, as long as no other `&mut self` method of this trait
///   runs in between.
/// - When the rule above promises room, `push_unchecked` must append the item
///   at the end.
/// - `clear` must drop every item and leave the storage empty. It must leave
///   the storage empty even when dropping an item panics.
/// - Only [`with_capacity`](WithCapacity::with_capacity), `ensure_room` and
///   `clear` may panic, and `clear` only when dropping an item panics. The
///   maps call the other methods partway through changes that a panic would
///   leave half done.
/// - [`empty`](Self::empty) must return a storage with no items, and so must
///   `with_capacity` if the storage implements [`WithCapacity`].
/// - If the storage implements `IntoIterator<Item = Self::Item>`,
///   `into_iter` must yield the same items as `as_slice`, in the same order.
/// - If that iterator also implements `DoubleEndedIterator` and
///   `ExactSizeIterator`, `next_back` must yield the items starting from the
///   last one, and `len` must be the number of items not yet yielded.
/// - If the storage implements `Clone`, the storage that `clone` returns must
///   hold a clone of each item, in the same order, as if each clone had been
///   pushed into an empty storage. `clone_from` must leave the storage holding
///   a clone of each item of the source in the same way. The maps clone their
///   storages with these methods.
pub unsafe trait SliceStorage {
    /// The items the storage holds. A `GenMap`'s storage holds its
    /// [`MapSlot`](crate::MapSlot)s, and a `SecondaryMap`'s holds its
    /// [`SecondaryMapSlot`](crate::SecondaryMapSlot)s.
    type Item;

    /// Why the storage could not make room for another item.
    type Error: fmt::Debug;

    /// Creates a storage with no items.
    fn empty() -> Self;

    /// How many items the storage can hold before it has to grow, or in
    /// total if it cannot grow.
    fn capacity(&self) -> usize;

    /// Returns every item as a slice, in the order the items were pushed.
    fn as_slice(&self) -> &[Self::Item];

    /// Returns every item as a mutable slice, in the order the items were
    /// pushed.
    fn as_mut_slice(&mut self) -> &mut [Self::Item];

    /// Makes sure the storage has room for the next `additional` calls of
    /// [`push_unchecked`](Self::push_unchecked), growing if the storage can
    /// and has to. A `SecondaryMap` asks for room up to a key's index, so
    /// `additional` can be larger than any storage can hold. The map expects
    /// an error in that case, not a panic.
    ///
    /// # Errors
    ///
    /// Returns why the storage cannot make room for all of them, when it
    /// cannot.
    fn ensure_room(&mut self, additional: usize) -> Result<(), Self::Error>;

    /// Appends `item` without checking that the storage has room for it.
    ///
    /// # Safety
    ///
    /// The rules of this trait must promise room for this call.
    unsafe fn push_unchecked(&mut self, item: Self::Item);

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
/// it needs more room. A map has `reserve` and `try_reserve` only when its
/// slot storage implements this trait. A dense map also needs
/// [`ReservePairStorage`](crate::ReservePairStorage) on its pair storage,
/// which a [`SplitPair`](crate::SplitPair) implements when both of its
/// storages implement this trait.
pub trait ReserveStorage: SliceStorage {}

/// A storage implements `WithCapacity` when it can be created with room for a
/// number of items chosen at runtime. A storage whose type sets its capacity,
/// such as an `ArrayVec`, does not implement it. A map has
/// `with_capacity_and_config` only when its storages implement this trait.
///
/// The rules of [`SliceStorage`] and [`PairStorage`](crate::PairStorage)
/// require `with_capacity` to return a storage with no items.
pub trait WithCapacity {
    /// Creates a storage with no items and room for at least `capacity` of
    /// them. A pair storage gets room for at least `capacity` pairs.
    fn with_capacity(capacity: usize) -> Self;
}

/// Empties a storage when it is dropped. A `clone_from` holds one for each
/// storage it clones into and forgets it once the cloning has finished, so a
/// `ClearOnUnwind` only empties a storage that a panic left half cloned. A half
/// cloned storage would disagree with the rest of its map, or with the other
/// storage of its `SplitPair`.
pub(crate) struct ClearOnUnwind<'a, St: SliceStorage>(pub(crate) &'a mut St);

impl<St: SliceStorage> Drop for ClearOnUnwind<'_, St> {
    fn drop(&mut self) {
        self.0.clear();
    }
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
// SAFETY: a `Vec` behaves exactly as the trait describes. `ensure_room` goes
// through `try_reserve`, which reports capacity overflow and allocation failure
// as errors instead of panicking or aborting. Once `try_reserve` has made room
// for `additional` items, the next `additional` pushes fit without growing.
// `push_unchecked` writes the item at the end without a check, and the rules
// only promise room while the capacity is above the length. Cloning a `Vec`
// with `clone` or `clone_from` gives one that holds a clone of each item, in
// the same order.
unsafe impl<S> SliceStorage for Vec<S> {
    type Item = S;
    type Error = TryReserveError;

    #[inline]
    fn empty() -> Self {
        Vec::new()
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
    unsafe fn push_unchecked(&mut self, item: S) {
        let len = Vec::len(self);
        debug_assert!(len < Vec::capacity(self));
        // SAFETY: the caller guarantees that the rules of the trait promise
        // room for this call, and a `Vec` only promises room while its capacity
        // is above its length. So position `len` is inside the allocation, and
        // `set_len` counts the item only after it is written.
        unsafe {
            self.as_mut_ptr().add(len).write(item);
            self.set_len(len + 1);
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

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
impl<S> WithCapacity for Vec<S> {
    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        Vec::with_capacity(capacity)
    }
}

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
// `Vec`. `ensure_room` returns `Ok` only when it has room for all
// `additional` items. `push_unchecked` writes the item without a check, and
// the rules only promise room while the `ArrayVec` is not full. Its `clear`
// sets the length to zero before it drops the items, so the storage is empty
// even when a drop panics. Cloning an `ArrayVec` with `clone` or `clone_from`
// gives one that holds a clone of each item, in the same order.
unsafe impl<S, const CAP: usize> SliceStorage for arrayvec::ArrayVec<S, CAP> {
    type Item = S;
    type Error = arrayvec::CapacityError;

    #[inline]
    fn empty() -> Self {
        arrayvec::ArrayVec::new()
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
    unsafe fn push_unchecked(&mut self, item: S) {
        // SAFETY: the caller guarantees that the rules of the trait promise
        // room for this call, and an `ArrayVec` only promises room while it is
        // not full.
        unsafe { arrayvec::ArrayVec::push_unchecked(self, item) };
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
// on the heap. `ensure_room` goes through `try_reserve`, which returns an
// error where growing would panic or abort. Once `try_reserve` has made room
// for `additional` items, the next `additional` pushes fit without growing.
// `push_unchecked` calls `SmallVec::push`, which only grows a full `SmallVec`,
// and the rules only promise room while it is not full, so that call never
// grows or panics. Cloning a `SmallVec` with `clone` or `clone_from` gives one
// that holds a clone of each item, in the same order.
unsafe impl<S, const N: usize> SliceStorage for smallvec::SmallVec<S, N> {
    type Item = S;
    type Error = smallvec::SmallVecError;

    #[inline]
    fn empty() -> Self {
        smallvec::SmallVec::new()
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
    unsafe fn push_unchecked(&mut self, item: S) {
        smallvec::SmallVec::push(self, item);
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

#[cfg(feature = "smallvec")]
#[cfg_attr(docsrs, doc(cfg(feature = "smallvec")))]
impl<S, const N: usize> WithCapacity for smallvec::SmallVec<S, N> {
    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        smallvec::SmallVec::with_capacity(capacity)
    }
}
