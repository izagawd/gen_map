use crate::error::ReserveError;
use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::mem::MaybeUninit;
use core::ptr;

/// The collection a [`GenMap`](crate::GenMap) keeps its slots in. A
/// [`Config`](crate::Config) names one through its `Storage` type.
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
/// called in between. [`push`](Self::push) must append at the end and leave
/// the other items where they are, and it must succeed whenever
/// [`is_full`](Self::is_full) has just returned `false`.
/// [`truncate`](Self::truncate) must drop only the items past the given
/// length. [`EMPTY`](Self::EMPTY) and [`with_capacity`](Self::with_capacity)
/// must hold no items.
pub unsafe trait SlotStorage<S>:
    IntoIterator<Item = S, IntoIter: DoubleEndedIterator + ExactSizeIterator>
{
    /// A storage with no items. It is a constant rather than a function so
    /// that a map can be created in a `const` context.
    const EMPTY: Self;

    /// Creates a storage with no items and room for `capacity` of them. A
    /// storage with a fixed capacity ignores the argument.
    fn with_capacity(capacity: usize) -> Self;

    /// How many items the storage can hold before it has to grow, or in
    /// total if it can not grow.
    fn capacity(&self) -> usize;

    /// Returns `true` if [`push`](Self::push) would fail. A storage that can
    /// grow always returns `false`.
    fn is_full(&self) -> bool;

    /// Every item, in the order they were pushed.
    fn as_slice(&self) -> &[S];

    /// Every item mutably, in the order they were pushed.
    fn as_mut_slice(&mut self) -> &mut [S];

    /// Appends `item`, or hands it back if the storage is full and can not
    /// grow.
    fn push(&mut self, item: S) -> Result<(), S>;

    /// Drops every item past the first `len` of them. Does nothing if there
    /// are no more than `len` items.
    fn truncate(&mut self, len: usize);

    /// Makes room for at least `additional` more items. A storage with a
    /// fixed capacity does nothing, and a later `push` past its capacity
    /// still fails.
    fn reserve(&mut self, additional: usize);

    /// The fallible form of [`reserve`](Self::reserve). A storage with a
    /// fixed capacity reports [`ReserveError::CapacityExceeded`] when the
    /// items would not fit.
    fn try_reserve(&mut self, additional: usize) -> Result<(), ReserveError>;

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

// SAFETY: `Vec` is the behaviour the trait describes.
unsafe impl<S> SlotStorage<S> for Vec<S> {
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
    fn is_full(&self) -> bool {
        false
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
    fn push(&mut self, item: S) -> Result<(), S> {
        Vec::push(self, item);
        Ok(())
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        Vec::truncate(self, len);
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        Vec::reserve(self, additional);
    }

    #[inline]
    fn try_reserve(&mut self, additional: usize) -> Result<(), ReserveError> {
        Vec::try_reserve(self, additional).map_err(ReserveError::Alloc)
    }
}
