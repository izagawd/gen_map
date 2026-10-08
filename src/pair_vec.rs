use crate::buffer::{allocate, deallocate, grown_capacity, max_len, saturating_usize};
use crate::error::ReserveError;
use crate::key_piece::KeyPiece;
use crate::pair_storage::{PairStorage, ReservePairStorage};
use core::fmt;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::mem::{self, size_of, ManuallyDrop};
use core::ptr::{self, NonNull};
use core::slice;

/// A [`PairStorage`] that keeps its two slices in two buffers on the heap.
/// Both buffers always have room for the same number of items, so a
/// `PairVec` stores one length and one capacity for both buffers. It stores
/// them as its length type, the third type parameter, which works the same
/// way as the length type of a [`SingleVec`](crate::SingleVec). The length
/// type is `usize` when it is left out.
///
/// [`DefaultMapConfig`](crate::DefaultMapConfig) keeps the keys and values of
/// the dense maps in a `PairVec` whose length type is the index type of the
/// keys. A `PairVec` needs the `alloc` feature.
///
/// A slice whose items take no space needs no buffer. When the items of
/// both slices take no space, a `PairVec` never allocates, and its capacity
/// is the most pairs its length type can count.
///
/// A `PairVec` drops its items in its own `Drop` implementation, so the
/// compiler requires everything its items borrow to outlive it.
///
/// # Examples
///
/// ```
/// use gen_map::{PairStorage, PairVec};
///
/// let mut pairs = PairVec::<i32, &str>::new();
/// pairs.push(1, "one");
/// pairs.push(2, "two");
/// assert_eq!(pairs.first_slice(), [1, 2]);
/// assert_eq!(pairs.second_slice(), ["one", "two"]);
/// assert_eq!(pairs.pop(), Some((2, "two")));
/// assert_eq!(pairs.len(), 1);
/// ```
pub struct PairVec<A, B, L: KeyPiece = usize> {
    buffers: Buffers<A, B, L>,
    /// A `PairVec` owns the items in its buffers.
    _items: PhantomData<(A, B)>,
}

/// The two buffers of a [`PairVec`] or a [`PairVecIntoIter`], together with
/// their capacity and how many of their items are initialized. Dropping a
/// `Buffers` frees both buffers without dropping any item in them.
struct Buffers<A, B, L: KeyPiece> {
    first: NonNull<A>,
    second: NonNull<B>,
    /// How many items each buffer has room for. When the items of both
    /// buffers take no space, it is the largest value of the length type.
    capacity: L,
    /// The number of pairs. The first `len` items of each buffer are
    /// initialized, and the rest are not. It is never more than the capacity
    /// or the largest `usize`.
    len: L,
}

/// Returns `true` if the items of both slices take no space, so that neither
/// slice needs a buffer.
#[inline]
const fn takes_no_space<A, B>() -> bool {
    size_of::<A>() == 0 && size_of::<B>() == 0
}

/// Drops the first `len` items of both buffers. If dropping an item of the
/// first buffer panics, the items of the second buffer are still dropped
/// while the panic unwinds.
///
/// # Safety
///
/// The first `len` items of both buffers must be initialized, and nothing may
/// use them afterwards.
unsafe fn drop_items<A, B>(first: *mut A, second: *mut B, len: usize) {
    /// Drops the items in `items` when it goes out of scope.
    struct DropOnExit<T> {
        items: *mut [T],
    }

    impl<T> Drop for DropOnExit<T> {
        fn drop(&mut self) {
            // SAFETY: the caller of `drop_items` promises that these items
            // are initialized and that nothing uses them afterwards.
            unsafe { ptr::drop_in_place(self.items) };
        }
    }

    let _second = DropOnExit {
        items: ptr::slice_from_raw_parts_mut(second, len),
    };
    // SAFETY: the caller promises that these items are initialized and that
    // nothing uses them afterwards.
    unsafe { ptr::drop_in_place(ptr::slice_from_raw_parts_mut(first, len)) };
}

impl<A, B, L: KeyPiece> Buffers<A, B, L> {
    /// Returns buffers that hold no memory. Their capacity is zero, or the
    /// largest value of the length type when the items of both slices take
    /// no space.
    #[inline]
    const fn new() -> Self {
        Self {
            first: NonNull::dangling(),
            second: NonNull::dangling(),
            capacity: if takes_no_space::<A, B>() {
                L::MAX
            } else {
                L::ZERO
            },
            len: L::ZERO,
        }
    }

    /// Allocates two buffers with room for `capacity` items each.
    fn with_capacity(capacity: usize) -> Result<Self, ReserveError> {
        let stored = L::from_usize(capacity).ok_or(ReserveError::CapacityOverflow)?;
        if takes_no_space::<A, B>() {
            return Ok(Self::new());
        }
        let first = allocate::<A>(capacity)?;
        let second = match allocate::<B>(capacity) {
            Ok(second) => second,
            Err(error) => {
                // SAFETY: `first` came from `allocate` for `capacity` items,
                // and nothing uses it after this.
                unsafe { deallocate(first, capacity) };
                return Err(error);
            }
        };
        Ok(Self {
            first,
            second,
            capacity: stored,
            len: L::ZERO,
        })
    }

    /// The number of pairs as a `usize`.
    #[inline]
    fn len(&self) -> usize {
        saturating_usize(self.len)
    }

    /// How many items each buffer has room for, as a `usize`. A capacity
    /// larger than the largest `usize` counts as the largest `usize`.
    #[inline]
    fn capacity(&self) -> usize {
        saturating_usize(self.capacity)
    }
}

impl<A, B, L: KeyPiece> Drop for Buffers<A, B, L> {
    fn drop(&mut self) {
        let capacity = self.capacity();
        // SAFETY: each buffer either came from `allocate` for `capacity`
        // items, or is for `capacity` items that take no space. Nothing uses
        // the buffers after this.
        unsafe {
            deallocate(self.first, capacity);
            deallocate(self.second, capacity);
        }
    }
}

impl<A, B, L: KeyPiece> PairVec<A, B, L> {
    /// Creates a `PairVec` with no pairs. It allocates nothing until a pair
    /// is pushed.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffers: Buffers::new(),
            _items: PhantomData,
        }
    }

    /// Creates a `PairVec` with no pairs and room for at least `capacity` of
    /// them.
    ///
    /// # Panics
    ///
    /// Panics if the length type cannot count `capacity` pairs, or if a
    /// buffer with room for `capacity` items would take more than
    /// `isize::MAX` bytes. If an allocation fails, `with_capacity` calls
    /// `handle_alloc_error`, which aborts the program by default.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        match Buffers::with_capacity(capacity) {
            Ok(buffers) => Self {
                buffers,
                _items: PhantomData,
            },
            Err(error) => error.handle(),
        }
    }

    /// Appends `first` to the first slice and `second` to the second slice, as
    /// one pair.
    ///
    /// # Panics
    ///
    /// Panics if the `PairVec` would hold more pairs than its length type can
    /// count, or need a buffer that takes more than `isize::MAX` bytes. If an
    /// allocation fails, `push` calls `handle_alloc_error`, which aborts the
    /// program by default.
    #[inline]
    pub fn push(&mut self, first: A, second: B) {
        if let Err(error) = self.make_room(1) {
            error.handle();
        }
        // SAFETY: `make_room` just made room for one more pair.
        unsafe { self.push_unchecked(first, second) };
    }

    /// Appends `first` to the first slice and `second` to the second slice, as
    /// one pair.
    ///
    /// # Errors
    ///
    /// Hands both items back if the `PairVec` cannot make room for them.
    #[inline]
    pub fn try_push(&mut self, first: A, second: B) -> Result<(), (A, B)> {
        if self.make_room(1).is_err() {
            return Err((first, second));
        }
        // SAFETY: `make_room` just made room for one more pair.
        unsafe { self.push_unchecked(first, second) };
        Ok(())
    }

    /// Makes room for at least `additional` more pairs. When the buffers are
    /// too small, it moves the pairs into new buffers with at least twice the
    /// old capacity.
    fn make_room(&mut self, additional: usize) -> Result<(), ReserveError> {
        let len = self.buffers.len();
        let required = len
            .checked_add(additional)
            .ok_or(ReserveError::CapacityOverflow)?;
        if required <= self.buffers.capacity() {
            return Ok(());
        }
        let capacity = grown_capacity(
            self.buffers.capacity(),
            required,
            max_len::<L>(),
            size_of::<A>().max(size_of::<B>()),
        )?;
        let mut buffers = Buffers::with_capacity(capacity)?;
        // SAFETY: the new buffers have room for more than `len` items each,
        // and they do not overlap the old ones. The pairs move into the new
        // buffers, and the old buffers are freed without dropping the pairs.
        unsafe {
            ptr::copy_nonoverlapping(self.buffers.first.as_ptr(), buffers.first.as_ptr(), len);
            ptr::copy_nonoverlapping(self.buffers.second.as_ptr(), buffers.second.as_ptr(), len);
        }
        buffers.len = self.buffers.len;
        mem::swap(&mut self.buffers, &mut buffers);
        Ok(())
    }

    /// Appends a pair without checking that there is room for it.
    ///
    /// # Safety
    ///
    /// The length must be below the capacity.
    #[inline]
    unsafe fn push_unchecked(&mut self, first: A, second: B) {
        let len = self.buffers.len();
        debug_assert!(len < self.buffers.capacity());
        // SAFETY: the caller promises that the length is below the capacity,
        // so both buffers have room for an item at position `len`.
        unsafe {
            self.buffers.first.as_ptr().add(len).write(first);
            self.buffers.second.as_ptr().add(len).write(second);
        }
        // The length is below the capacity, which is a value of the length
        // type, so adding one does not wrap.
        self.buffers.len = self.buffers.len.wrapping_add(L::ONE);
    }
}

impl<A, B, L: KeyPiece> Default for PairVec<A, B, L> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<A, B, L: KeyPiece> Drop for PairVec<A, B, L> {
    fn drop(&mut self) {
        // SAFETY: the first `len` items of both buffers are initialized, and
        // nothing uses them after this. `buffers` frees the memory afterwards,
        // even when dropping an item panics.
        unsafe {
            drop_items(
                self.buffers.first.as_ptr(),
                self.buffers.second.as_ptr(),
                self.buffers.len(),
            );
        }
    }
}

// SAFETY: the first `len` items of both buffers are initialized and belong to
// the pairs, in the order they were pushed, and both slices hold `len`
// items, so they always have the same length. `try_push` writes both items
// only once there is room for them, `pop` and `clear` lower the length before
// they move or drop an item, and growing copies the items into the new
// buffers without changing them. Once `ensure_room` returns `Ok` for `n`
// pairs, the capacity is at least the length plus `n`, so the next `n`
// pushes fit without growing. The two buffers are separate allocations or
// hold no memory, so the two mutable slices never overlap.
unsafe impl<A, B, L: KeyPiece> PairStorage for PairVec<A, B, L> {
    type First = A;
    type Second = B;
    type Error = ReserveError;

    #[inline]
    fn empty() -> Self {
        Self::new()
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        PairVec::with_capacity(capacity)
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.buffers.capacity()
    }

    #[inline]
    fn first_slice(&self) -> &[A] {
        // SAFETY: the first `len` items of the first buffer are initialized.
        unsafe { slice::from_raw_parts(self.buffers.first.as_ptr(), self.buffers.len()) }
    }

    #[inline]
    fn second_slice(&self) -> &[B] {
        // SAFETY: the first `len` items of the second buffer are initialized.
        unsafe { slice::from_raw_parts(self.buffers.second.as_ptr(), self.buffers.len()) }
    }

    #[inline]
    fn slices(&self) -> (&[A], &[B]) {
        (self.first_slice(), self.second_slice())
    }

    #[inline]
    fn first_slice_mut(&mut self) -> &mut [A] {
        // SAFETY: the first `len` items of the first buffer are initialized,
        // and `&mut self` keeps any other reference to them from existing.
        unsafe { slice::from_raw_parts_mut(self.buffers.first.as_ptr(), self.buffers.len()) }
    }

    #[inline]
    fn second_slice_mut(&mut self) -> &mut [B] {
        // SAFETY: the same as in `first_slice_mut`, for the second buffer.
        unsafe { slice::from_raw_parts_mut(self.buffers.second.as_ptr(), self.buffers.len()) }
    }

    #[inline]
    fn slices_mut(&mut self) -> (&mut [A], &mut [B]) {
        let len = self.buffers.len();
        // SAFETY: the same as in `first_slice_mut`. The two buffers are
        // separate allocations or hold no memory, so the two slices do not
        // overlap.
        unsafe {
            (
                slice::from_raw_parts_mut(self.buffers.first.as_ptr(), len),
                slice::from_raw_parts_mut(self.buffers.second.as_ptr(), len),
            )
        }
    }

    #[inline]
    fn ensure_room(&mut self, additional: usize) -> Result<(), ReserveError> {
        self.make_room(additional)
    }

    #[inline]
    fn try_push(&mut self, first: A, second: B) -> Result<(), (A, B)> {
        PairVec::try_push(self, first, second)
    }

    #[inline]
    fn pop(&mut self) -> Option<(A, B)> {
        if self.buffers.len == L::ZERO {
            return None;
        }
        self.buffers.len = self.buffers.len.wrapping_sub(L::ONE);
        let len = self.buffers.len();
        // SAFETY: the items at the new length were the initialized last pair,
        // and lowering the length gave up ownership of them.
        unsafe {
            Some((
                self.buffers.first.as_ptr().add(len).read(),
                self.buffers.second.as_ptr().add(len).read(),
            ))
        }
    }

    #[inline]
    fn clear(&mut self) {
        let len = self.buffers.len();
        self.buffers.len = L::ZERO;
        // SAFETY: the first `len` items of both buffers were initialized, and
        // setting the length to zero gave up ownership of them, so the storage
        // is empty even when dropping an item panics.
        unsafe {
            drop_items(
                self.buffers.first.as_ptr(),
                self.buffers.second.as_ptr(),
                len,
            );
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.buffers.len()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.buffers.len == L::ZERO
    }
}

impl<A, B, L: KeyPiece> ReservePairStorage for PairVec<A, B, L> {}

// SAFETY: a `PairVec` owns its items the way a `Vec` does, so it can move to
// another thread when its items can.
unsafe impl<A: Send, B: Send, L: KeyPiece> Send for PairVec<A, B, L> {}

// SAFETY: a shared `PairVec` only hands out shared references to its items,
// so it can be shared between threads when its items can.
unsafe impl<A: Sync, B: Sync, L: KeyPiece> Sync for PairVec<A, B, L> {}

impl<A: Clone, B: Clone, L: KeyPiece> Clone for PairVec<A, B, L> {
    fn clone(&self) -> Self {
        let mut clone = Self::with_capacity(self.buffers.len());
        for (first, second) in self.first_slice().iter().zip(self.second_slice()) {
            clone.push(first.clone(), second.clone());
        }
        clone
    }
}

impl<A: fmt::Debug, B: fmt::Debug, L: KeyPiece> fmt::Debug for PairVec<A, B, L> {
    /// Lists the pairs in the order they were pushed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.first_slice().iter().zip(self.second_slice()))
            .finish()
    }
}

impl<A, B, L: KeyPiece> FromIterator<(A, B)> for PairVec<A, B, L> {
    fn from_iter<I: IntoIterator<Item = (A, B)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut pairs = Self::with_capacity(iter.size_hint().0);
        for (first, second) in iter {
            pairs.push(first, second);
        }
        pairs
    }
}

impl<A, B, L: KeyPiece> IntoIterator for PairVec<A, B, L> {
    type Item = (A, B);
    type IntoIter = PairVecIntoIter<A, B, L>;

    #[inline]
    fn into_iter(self) -> PairVecIntoIter<A, B, L> {
        let pairs = ManuallyDrop::new(self);
        // SAFETY: `pairs` is never dropped, so the buffers move into the
        // iterator and are only freed once, by the iterator.
        let buffers = unsafe { ptr::read(&pairs.buffers) };
        PairVecIntoIter {
            buffers,
            start: 0,
            _items: PhantomData,
        }
    }
}

/// Owning iterator over the pairs of a [`PairVec`], in the order they were
/// pushed. It is created by consuming a `PairVec` with `into_iter`. Dropping
/// it drops the pairs it has not yielded yet.
pub struct PairVecIntoIter<A, B, L: KeyPiece = usize> {
    /// The buffers of the `PairVec`. Their length is one past the position of
    /// the pair that `next_back` yields.
    buffers: Buffers<A, B, L>,
    /// The position of the pair that `next` yields. The pairs from `start` up
    /// to the length of the buffers are initialized and not yielded yet.
    start: usize,
    /// The iterator owns the pairs it has not yielded yet.
    _items: PhantomData<(A, B)>,
}

impl<A, B, L: KeyPiece> PairVecIntoIter<A, B, L> {
    /// Reads the pair at `position` out of the buffers.
    ///
    /// # Safety
    ///
    /// The pair at `position` must be initialized, and the iterator must give
    /// up ownership of it, so that nothing reads or drops it again.
    #[inline]
    unsafe fn read(&self, position: usize) -> (A, B) {
        // SAFETY: the caller promises that the pair at `position` is
        // initialized and that nothing reads or drops it again.
        unsafe {
            (
                self.buffers.first.as_ptr().add(position).read(),
                self.buffers.second.as_ptr().add(position).read(),
            )
        }
    }
}

impl<A, B, L: KeyPiece> Iterator for PairVecIntoIter<A, B, L> {
    type Item = (A, B);

    #[inline]
    fn next(&mut self) -> Option<(A, B)> {
        if self.start == self.buffers.len() {
            return None;
        }
        let position = self.start;
        self.start += 1;
        // SAFETY: the pair at `position` was not yielded yet, so it is
        // initialized, and raising `start` past it gave up ownership of it.
        Some(unsafe { self.read(position) })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.buffers.len() - self.start;
        (len, Some(len))
    }
}

impl<A, B, L: KeyPiece> DoubleEndedIterator for PairVecIntoIter<A, B, L> {
    #[inline]
    fn next_back(&mut self) -> Option<(A, B)> {
        if self.start == self.buffers.len() {
            return None;
        }
        self.buffers.len = self.buffers.len.wrapping_sub(L::ONE);
        // SAFETY: the pair at the new length was not yielded yet, so it is
        // initialized, and lowering the length to it gave up ownership of it.
        Some(unsafe { self.read(self.buffers.len()) })
    }
}

impl<A, B, L: KeyPiece> ExactSizeIterator for PairVecIntoIter<A, B, L> {}
impl<A, B, L: KeyPiece> FusedIterator for PairVecIntoIter<A, B, L> {}

impl<A, B, L: KeyPiece> Drop for PairVecIntoIter<A, B, L> {
    fn drop(&mut self) {
        let remaining = self.buffers.len() - self.start;
        // SAFETY: `start` is never more than the length, which is never more
        // than the capacity, so both pointers stay inside their buffers or one
        // past their ends. The pairs from `start` up to the length are
        // initialized and not yielded yet, and nothing uses them after this.
        // `buffers` frees the memory afterwards, even when dropping an item
        // panics.
        unsafe {
            drop_items(
                self.buffers.first.as_ptr().add(self.start),
                self.buffers.second.as_ptr().add(self.start),
                remaining,
            );
        }
    }
}

// SAFETY: the iterator owns the pairs it has not yielded yet, the way a
// `PairVec` owns its pairs.
unsafe impl<A: Send, B: Send, L: KeyPiece> Send for PairVecIntoIter<A, B, L> {}

// SAFETY: a shared iterator hands out no references to its pairs.
unsafe impl<A: Sync, B: Sync, L: KeyPiece> Sync for PairVecIntoIter<A, B, L> {}
