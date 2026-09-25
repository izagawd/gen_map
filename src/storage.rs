#[cfg(feature = "alloc")]
use alloc::collections::TryReserveError;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::fmt;

/// The collection a [`GenMap`](crate::GenMap) keeps its slots in. A
/// [`MapConfig`](crate::MapConfig) names one through its `Storage` type.
///
/// A storage that can also grow on request implements [`ReserveStorage`]
/// too.
///
/// # Safety
///
/// The map reads slots without bounds checks at positions it has already
/// checked against [`len`](Self::len), and it keeps a free list of positions
/// that it expects to still be there. So a storage must behave like a `Vec`
/// in these ways. [`as_slice`](Self::as_slice) and
/// [`as_mut_slice`](Self::as_mut_slice) must return every item that has been
/// pushed since the last [`clear`](Self::clear), in the order they were
/// pushed, and must return the same items every time unless a `&mut self`
/// method of this trait was called in between. [`try_push`](Self::try_push)
/// must append at the end and leave the other items where they are, and once
/// [`ensure_room`](Self::ensure_room) has returned `Ok`, the next `try_push`
/// must succeed, as long as no other `&mut self` method of this trait runs
/// in between. `clear` must drop every item and leave the storage empty.
/// [`EMPTY`](Self::EMPTY) and [`with_capacity`](Self::with_capacity) must
/// hold no items.
pub unsafe trait SlotStorage<S>:
    IntoIterator<Item = S, IntoIter: DoubleEndedIterator + ExactSizeIterator>
{
    /// Why the storage could not make room for another item.
    type Error: fmt::Debug;

    /// A storage with no items. It is a constant rather than a function so
    /// that a map can be created in a `const` context.
    const EMPTY: Self;

    /// Creates a storage with no items and, if it can grow, room for
    /// `capacity` of them. A storage with a fixed capacity ignores the
    /// argument.
    fn with_capacity(capacity: usize) -> Self;

    /// How many items the storage can hold before it has to grow, or in
    /// total if it can not grow.
    fn capacity(&self) -> usize;

    /// Every item, in the order they were pushed.
    fn as_slice(&self) -> &[S];

    /// Every item mutably, in the order they were pushed.
    fn as_mut_slice(&mut self) -> &mut [S];

    /// Makes sure the next [`try_push`](Self::try_push) will succeed,
    /// growing if the storage can and has to, or returns the reason the push
    /// would fail.
    fn ensure_room(&mut self) -> Result<(), Self::Error>;

    /// Appends `item`, or hands it back if the storage can not make room for
    /// it.
    fn try_push(&mut self, item: S) -> Result<(), S>;

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

/// A [`SlotStorage`] that can make room for more items on request. After
/// [`try_reserve`](Self::try_reserve) has returned `Ok` for `n` more items,
/// the next `n` calls of [`try_push`](SlotStorage::try_push) succeed.
pub trait ReserveStorage<S>: SlotStorage<S> {
    /// Makes room for at least `additional` more items.
    ///
    /// # Panics
    ///
    /// Panics or aborts if the room can not be made, as `Vec::reserve` does.
    /// Use [`try_reserve`](Self::try_reserve) to get an error instead.
    fn reserve(&mut self, additional: usize);

    /// The fallible form of [`reserve`](Self::reserve).
    fn try_reserve(&mut self, additional: usize) -> Result<(), Self::Error>;
}

// SAFETY: `Vec` is the behaviour the trait describes. `Vec::push` itself
// panics on capacity overflow and aborts on allocation failure, so both
// `ensure_room` and `try_push` go through `try_reserve`, which reports the
// two as errors.
#[cfg(feature = "alloc")]
unsafe impl<S> SlotStorage<S> for Vec<S> {
    type Error = TryReserveError;

    const EMPTY: Self = Vec::new();

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
    fn ensure_room(&mut self) -> Result<(), TryReserveError> {
        Vec::try_reserve(self, 1)
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
    fn clear(&mut self) {
        Vec::clear(self)
    }
}

#[cfg(feature = "alloc")]
impl<S> ReserveStorage<S> for Vec<S> {
    #[inline]
    fn reserve(&mut self, additional: usize) {
        Vec::reserve(self, additional);
    }

    #[inline]
    fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        Vec::try_reserve(self, additional)
    }
}

/// Storage from the `arrayvec` crate that keeps up to `CAP` slots inline
/// and never allocates. It needs the `arrayvec` feature.
///
/// The storage can not grow, so once all `CAP` slots exist and none of them
/// are free, inserting fails with
/// [`FullError::StorageFull`](crate::FullError::StorageFull).
///
/// ```
/// use arrayvec::ArrayVec;
/// use gen_map::{GenMap, KeyConfig, MapConfig, Split};
///
/// /// Room for sixteen slots, without any allocation.
/// struct Inline;
///
/// impl KeyConfig for Inline {
///     type Idx = u8;
///     type Gen = u8;
///     type Layout = Split;
/// }
///
/// impl MapConfig for Inline {
///     type KeyConfig = Self;
///     type Storage<S> = ArrayVec<S, 16>;
/// }
///
/// let mut map = GenMap::<u32, Inline>::new_with_config();
/// let key = map.insert(1);
/// assert_eq!(map[key], 1);
/// assert_eq!(map.capacity(), 16);
/// ```
#[cfg(feature = "arrayvec")]
// SAFETY: an `ArrayVec` keeps its items in order and in place, like a
// `Vec`. Its `try_push` only fails when it is full, and `ensure_room`
// returns `Ok` only when it is not.
unsafe impl<S, const CAP: usize> SlotStorage<S> for arrayvec::ArrayVec<S, CAP> {
    type Error = arrayvec::CapacityError;

    const EMPTY: Self = arrayvec::ArrayVec::new_const();

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
    fn ensure_room(&mut self) -> Result<(), arrayvec::CapacityError> {
        if arrayvec::ArrayVec::is_full(self) {
            Err(arrayvec::CapacityError::new(()))
        } else {
            Ok(())
        }
    }

    #[inline]
    fn try_push(&mut self, item: S) -> Result<(), S> {
        arrayvec::ArrayVec::try_push(self, item).map_err(arrayvec::CapacityError::element)
    }

    #[inline]
    fn clear(&mut self) {
        arrayvec::ArrayVec::clear(self);
    }
}

/// Storage from the `smallvec` crate that keeps up to `N` slots inline and
/// moves them to the heap once there are more. It needs the `smallvec`
/// feature, which uses the 2.0 beta of `smallvec` and needs Rust 1.86.
/// Until smallvec 2.0 is released, a newer smallvec beta or a new release of
/// gen_map may break this feature, so it is not covered by semver.
///
/// ```
/// use gen_map::{GenMap, KeyConfig, MapConfig, Split};
/// use smallvec::SmallVec;
///
/// /// Room for eight slots before the map allocates.
/// struct Small;
///
/// impl KeyConfig for Small {
///     type Idx = u32;
///     type Gen = u32;
///     type Layout = Split;
/// }
///
/// impl MapConfig for Small {
///     type KeyConfig = Self;
///     type Storage<S> = SmallVec<S, 8>;
/// }
///
/// let mut map = GenMap::<u32, Small>::new_with_config();
/// let keys: Vec<_> = (0..20).map(|i| map.insert(i)).collect();
/// assert_eq!(map[keys[19]], 19);
/// ```
#[cfg(feature = "smallvec")]
// SAFETY: a `SmallVec` behaves like a `Vec` whether its items are inline or
// on the heap. `SmallVec::push` panics or aborts when it can not grow, so
// both `ensure_room` and `try_push` go through `try_reserve`, which returns
// an error instead.
unsafe impl<S, const N: usize> SlotStorage<S> for smallvec::SmallVec<S, N> {
    type Error = smallvec::CollectionAllocErr;

    const EMPTY: Self = smallvec::SmallVec::new();

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
    fn ensure_room(&mut self) -> Result<(), smallvec::CollectionAllocErr> {
        smallvec::SmallVec::try_reserve(self, 1)
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
    fn clear(&mut self) {
        smallvec::SmallVec::clear(self);
    }
}

#[cfg(feature = "smallvec")]
impl<S, const N: usize> ReserveStorage<S> for smallvec::SmallVec<S, N> {
    #[inline]
    fn reserve(&mut self, additional: usize) {
        smallvec::SmallVec::reserve(self, additional);
    }

    #[inline]
    fn try_reserve(&mut self, additional: usize) -> Result<(), smallvec::CollectionAllocErr> {
        smallvec::SmallVec::try_reserve(self, additional)
    }
}
