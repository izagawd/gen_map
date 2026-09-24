use alloc::collections::TryReserveError;
use alloc::vec::Vec;
use core::fmt;

/// The collection a [`GenMap`](crate::GenMap) keeps its slots in. A
/// [`Config`](crate::Config) names one through its `Storage` type.
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
/// pushed and not truncated, in the order they were pushed, and must return
/// the same items every time unless a `&mut self` method of this trait was
/// called in between. [`try_push`](Self::try_push) must append at the end
/// and leave the other items where they are, and once
/// [`ensure_room`](Self::ensure_room) has returned `Ok`, the next `try_push`
/// must succeed, as long as no other `&mut self` method of this trait runs
/// in between. [`truncate`](Self::truncate) must drop only the items past
/// the given length. [`EMPTY`](Self::EMPTY) and
/// [`with_capacity`](Self::with_capacity) must hold no items.
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
    /// growing if the storage can and has to, or says why it will not.
    fn ensure_room(&mut self) -> Result<(), Self::Error>;

    /// Appends `item`, or hands it back if the storage can not make room for
    /// it.
    fn try_push(&mut self, item: S) -> Result<(), S>;

    /// Drops every item past the first `len` of them. Does nothing if there
    /// are no more than `len` items.
    fn truncate(&mut self, len: usize);

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

    /// Drops every item.
    #[inline]
    fn clear(&mut self) {
        self.truncate(0);
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
    fn truncate(&mut self, len: usize) {
        Vec::truncate(self, len);
    }
}

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
