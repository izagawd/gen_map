use crate::error::ReserveError;
use crate::key::piece::KeyPiece;
use crate::storage::buffer::{
    allocate_layout, deallocate_layout, grown_capacity, max_len, reallocate_layout,
    saturating_usize,
};
use crate::storage::pair::{PairStorage, ReservePairStorage};
use core::alloc::Layout;
use core::fmt;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::mem::{size_of, ManuallyDrop};
use core::ptr::{self, NonNull};
use core::slice;

/// A `PairVec` is a [`PairStorage`] that keeps its two slices in one buffer
/// on the heap. The buffer holds the items of the first slice, followed by
/// the items of the second slice. Both slices always have room for the same
/// number of items, so a `PairVec` stores one length and one capacity for
/// both. It stores them as its length type, the third type parameter, which
/// works the same way as the length type of a [`SingleVec`](crate::SingleVec).
/// The length type is `usize` when it is left out.
///
/// With [`DefaultMapConfig`](crate::DefaultMapConfig), the dense maps keep
/// their keys and values in a `PairVec` whose length type is the index type
/// of the keys. A `PairVec` needs the `alloc` feature.
///
/// Items that take no space need no room in the buffer. When the items of
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
    buffer: Buffer<A, B, L>,
    /// A `PairVec` owns the items in its buffer.
    _items: PhantomData<(A, B)>,
}

/// A `Buffer` is the buffer of a [`PairVec`] or a [`PairVecIntoIter`],
/// together with its capacity and how many of its pairs are initialized.
/// Dropping a `Buffer` frees the memory without dropping any item in it.
///
/// [`buffer_layout`] gives the layout of the buffer for its capacity, and the
/// position in it where the second slice starts. When the buffer holds no
/// memory, both pointers are dangling.
struct Buffer<A, B, L: KeyPiece> {
    /// `first` points at the first slice, at the start of the buffer.
    first: NonNull<A>,
    /// `second` points at the second slice, which starts after the room for
    /// the first slice.
    second: NonNull<B>,
    /// `capacity` is how many items each slice has room for. When the items
    /// of both slices take no space, it is the largest value of the length
    /// type.
    capacity: L,
    /// `len` is the number of pairs. The first `len` items of each slice are
    /// initialized, and the rest are not. It is never more than the capacity
    /// or the largest `usize`.
    len: L,
}

/// Returns `true` if the items of both slices take no space, so that the
/// buffer never needs memory.
#[inline]
const fn takes_no_space<A, B>() -> bool {
    size_of::<A>() == 0 && size_of::<B>() == 0
}

/// Drops the first `len` items of both slices. If dropping an item of the
/// first slice panics, the items of the second slice are still dropped while
/// the panic unwinds.
///
/// # Safety
///
/// The first `len` items of both slices must be initialized, and nothing may
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

/// Returns the layout of a buffer with room for `capacity` items in each
/// slice, and how many bytes into the buffer the second slice starts. The
/// first slice starts at the beginning of the buffer.
///
/// # Errors
///
/// Returns [`ReserveError::CapacityOverflow`] if the buffer would take more
/// than `isize::MAX` bytes.
#[inline]
fn buffer_layout<A, B>(capacity: usize) -> Result<(Layout, usize), ReserveError> {
    let first = Layout::array::<A>(capacity).map_err(|_| ReserveError::CapacityOverflow)?;
    let second = Layout::array::<B>(capacity).map_err(|_| ReserveError::CapacityOverflow)?;
    first
        .extend(second)
        .map_err(|_| ReserveError::CapacityOverflow)
}

impl<A, B, L: KeyPiece> Buffer<A, B, L> {
    /// Returns a buffer that holds no memory. Its capacity is zero, or the
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

    /// Allocates a buffer with room for `capacity` items in each slice.
    fn with_capacity(capacity: usize) -> Result<Self, ReserveError> {
        let stored = L::from_usize(capacity).ok_or(ReserveError::CapacityOverflow)?;
        let (layout, offset) = buffer_layout::<A, B>(capacity)?;
        if layout.size() == 0 {
            return Ok(Self::new());
        }
        // SAFETY: the layout takes space.
        let start = unsafe { allocate_layout(layout)? };
        Ok(Self {
            first: start.cast(),
            // SAFETY: the second slice starts `offset` bytes into the buffer,
            // and that position is no further than the end of the buffer.
            second: unsafe { start.add(offset) }.cast(),
            capacity: stored,
            len: L::ZERO,
        })
    }

    /// The number of pairs as a `usize`.
    #[inline]
    fn len(&self) -> usize {
        saturating_usize(self.len)
    }

    /// Returns how many items each slice has room for, as a `usize`. A
    /// capacity larger than the largest `usize` counts as the largest `usize`.
    #[inline]
    fn capacity(&self) -> usize {
        saturating_usize(self.capacity)
    }
}

impl<A, B, L: KeyPiece> Drop for Buffer<A, B, L> {
    fn drop(&mut self) {
        // SAFETY: the buffer was allocated with the layout for its capacity,
        // or that layout takes no space, so making the layout again succeeds.
        let (layout, _) = unsafe { buffer_layout::<A, B>(self.capacity()).unwrap_unchecked() };
        // SAFETY: if the layout takes space, the global allocator allocated the
        // buffer with it, by `allocate_layout`, by `reallocate_layout` or as the
        // rules of `PairVecRawParts` require, and `first` points at the start
        // of the buffer. Nothing uses the buffer after this.
        unsafe { deallocate_layout(self.first.cast(), layout) };
    }
}

impl<A, B, L: KeyPiece> PairVec<A, B, L> {
    /// Creates a `PairVec` with no pairs. It allocates nothing until a pair
    /// is pushed.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            _items: PhantomData,
        }
    }

    /// Creates a `PairVec` with no pairs and room for at least `capacity` of
    /// them.
    ///
    /// # Panics
    ///
    /// Panics if the length type cannot count `capacity` pairs, or if a
    /// buffer with room for `capacity` pairs would take more than
    /// `isize::MAX` bytes. If an allocation fails, `with_capacity` calls
    /// `handle_alloc_error`, which aborts the program by default.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        match Buffer::with_capacity(capacity) {
            Ok(buffer) => Self {
                buffer,
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

    /// Makes room for at least `additional` more pairs, and grows the buffer
    /// when it is too small.
    #[inline]
    fn make_room(&mut self, additional: usize) -> Result<(), ReserveError> {
        let required = self
            .buffer
            .len()
            .checked_add(additional)
            .ok_or(ReserveError::CapacityOverflow)?;
        if required <= self.buffer.capacity() {
            return Ok(());
        }
        self.grow(required)
    }

    /// Grows the buffer so it has room for at least `required` pairs, which
    /// must be more than its capacity. The new capacity is at least twice the
    /// old one, unless the length type cannot count that many pairs. The items
    /// of the first slice stay at the start of the grown buffer, and this
    /// method moves the items of the second slice to where the second slice
    /// starts in it.
    #[inline(never)]
    fn grow(&mut self, required: usize) -> Result<(), ReserveError> {
        let old_capacity = self.buffer.capacity();
        let capacity = grown_capacity(
            old_capacity,
            required,
            max_len::<L>(),
            size_of::<A>().max(size_of::<B>()),
        )?;
        let stored = L::from_usize(capacity).ok_or(ReserveError::CapacityOverflow)?;
        let (layout, offset) = buffer_layout::<A, B>(capacity)?;
        // SAFETY: the buffer was allocated with the layout for its capacity,
        // or that layout takes no space, so making the layout again succeeds.
        let (old_layout, old_offset) =
            unsafe { buffer_layout::<A, B>(old_capacity).unwrap_unchecked() };
        // SAFETY: if `old_layout` takes space, the global allocator allocated
        // the buffer with it, by `allocate_layout`, by `reallocate_layout` or
        // as the rules of `PairVecRawParts` require, and `first` points at the
        // start of the buffer. Both layouts have the larger alignment of the
        // two item types. `grown_capacity` returns at least `required`, which
        // is more than `old_capacity`, and a `PairVec` whose items all take no
        // space never grows, so `layout` takes more space than `old_layout`.
        // The old pointers are overwritten below, so nothing uses them
        // afterwards.
        let start = unsafe { reallocate_layout(self.buffer.first.cast(), old_layout, layout)? };
        // SAFETY: `reallocate_layout` kept the bytes of the old buffer at the
        // start of the grown one, so the first `len` items of the second slice
        // start `old_offset` bytes into it. The second slice of the grown
        // buffer starts `offset` bytes into it and has room for `capacity`
        // items. `ptr::copy` moves the items even when the two ranges overlap.
        let second = unsafe {
            let second = start.add(offset).cast::<B>();
            ptr::copy(
                start.add(old_offset).cast::<B>().as_ptr(),
                second.as_ptr(),
                self.buffer.len(),
            );
            second
        };
        self.buffer.first = start.cast();
        self.buffer.second = second;
        self.buffer.capacity = stored;
        Ok(())
    }

    /// Appends a pair without checking that there is room for it.
    ///
    /// # Safety
    ///
    /// The length must be below the capacity.
    #[inline]
    unsafe fn push_unchecked(&mut self, first: A, second: B) {
        let len = self.buffer.len();
        debug_assert!(len < self.buffer.capacity());
        // SAFETY: the caller promises that the length is below the capacity,
        // so both slices have room for an item at position `len`.
        unsafe {
            self.buffer.first.as_ptr().add(len).write(first);
            self.buffer.second.as_ptr().add(len).write(second);
        }
        // The length is below the capacity, which is a value of the length
        // type, so adding one does not wrap.
        self.buffer.len = self.buffer.len.wrapping_add(L::ONE);
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
        // SAFETY: the first `len` items of both slices are initialized, and
        // nothing uses them after this. `buffer` frees the memory afterwards,
        // even when dropping an item panics.
        unsafe {
            drop_items(
                self.buffer.first.as_ptr(),
                self.buffer.second.as_ptr(),
                self.buffer.len(),
            );
        }
    }
}

// SAFETY: the first `len` items of both slices are initialized and belong to
// the pairs, in the order they were pushed, and both slices hold `len`
// items, so they always have the same length. `try_push` writes both items
// only once there is room for them, `pop` and `clear` lower the length before
// they move or drop an item, and growing moves the items without changing
// them. Once `ensure_room` returns `Ok` for `n` pairs, the capacity is at
// least the length plus `n`, so the next `n` pushes fit without growing. The
// `PairVec` that `with_capacity` returns has room for at least `capacity`
// pairs, so the first `capacity` pushes fit too. `push_unchecked` writes both
// items without a check, and the rules only promise that a push succeeds while
// the length is below the capacity. The first slice ends before the second
// slice starts in the buffer, so the two mutable slices never overlap. Only
// `with_capacity` panics on its own, and `clear` only panics when dropping an
// item does.
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
        self.buffer.capacity()
    }

    #[inline]
    fn first_slice(&self) -> &[A] {
        // SAFETY: the first `len` items of the first slice are initialized.
        unsafe { slice::from_raw_parts(self.buffer.first.as_ptr(), self.buffer.len()) }
    }

    #[inline]
    fn second_slice(&self) -> &[B] {
        // SAFETY: the first `len` items of the second slice are initialized.
        unsafe { slice::from_raw_parts(self.buffer.second.as_ptr(), self.buffer.len()) }
    }

    #[inline]
    fn slices(&self) -> (&[A], &[B]) {
        (self.first_slice(), self.second_slice())
    }

    #[inline]
    fn first_slice_mut(&mut self) -> &mut [A] {
        // SAFETY: the first `len` items of the first slice are initialized,
        // and `&mut self` keeps any other reference to them from existing.
        unsafe { slice::from_raw_parts_mut(self.buffer.first.as_ptr(), self.buffer.len()) }
    }

    #[inline]
    fn second_slice_mut(&mut self) -> &mut [B] {
        // SAFETY: the first `len` items of the second slice are initialized,
        // and `&mut self` keeps any other reference to them from existing.
        unsafe { slice::from_raw_parts_mut(self.buffer.second.as_ptr(), self.buffer.len()) }
    }

    #[inline]
    fn slices_mut(&mut self) -> (&mut [A], &mut [B]) {
        let len = self.buffer.len();
        // SAFETY: the first `len` items of both slices are initialized, and
        // `&mut self` keeps any other reference to them from existing. The
        // first slice ends before the second slice starts in the buffer, so the
        // two slices do not overlap.
        unsafe {
            (
                slice::from_raw_parts_mut(self.buffer.first.as_ptr(), len),
                slice::from_raw_parts_mut(self.buffer.second.as_ptr(), len),
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
    unsafe fn push_unchecked(&mut self, first: A, second: B) {
        // SAFETY: the caller promises that the rules of the trait make this
        // push succeed, and a `PairVec` only promises that while its length is
        // below its capacity.
        unsafe { PairVec::push_unchecked(self, first, second) };
    }

    #[inline]
    fn pop(&mut self) -> Option<(A, B)> {
        if self.buffer.len == L::ZERO {
            return None;
        }
        self.buffer.len = self.buffer.len.wrapping_sub(L::ONE);
        let len = self.buffer.len();
        // SAFETY: the items at the new length were the initialized last pair,
        // and lowering the length gave up ownership of them.
        unsafe {
            Some((
                self.buffer.first.as_ptr().add(len).read(),
                self.buffer.second.as_ptr().add(len).read(),
            ))
        }
    }

    #[inline]
    fn clear(&mut self) {
        let len = self.buffer.len();
        self.buffer.len = L::ZERO;
        // SAFETY: the first `len` items of both slices were initialized, and
        // setting the length to zero gave up ownership of them, so the storage
        // is empty even when dropping an item panics.
        unsafe {
            drop_items(self.buffer.first.as_ptr(), self.buffer.second.as_ptr(), len);
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.buffer.len()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.buffer.len == L::ZERO
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
        let mut clone = Self::with_capacity(self.buffer.len());
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
        // SAFETY: `pairs` is never dropped, so the buffer moves into the
        // iterator and is only freed once, by the iterator.
        let buffer = unsafe { ptr::read(&pairs.buffer) };
        PairVecIntoIter {
            buffer,
            start: 0,
            _items: PhantomData,
        }
    }
}

/// Owning iterator over the pairs of a [`PairVec`], in the order they were
/// pushed. It is created by consuming a `PairVec` with `into_iter`. Dropping
/// it drops the pairs it has not yielded yet.
pub struct PairVecIntoIter<A, B, L: KeyPiece = usize> {
    /// `buffer` holds the buffer of the `PairVec`. Its length is one past the
    /// position of the pair that `next_back` yields.
    buffer: Buffer<A, B, L>,
    /// `start` is the position of the pair that `next` yields. The pairs from
    /// `start` up to the length of the buffer are initialized and not yielded
    /// yet.
    start: usize,
    /// The iterator owns the pairs it has not yielded yet.
    _items: PhantomData<(A, B)>,
}

impl<A, B, L: KeyPiece> PairVecIntoIter<A, B, L> {
    /// Reads the pair at `position` out of the buffer.
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
                self.buffer.first.as_ptr().add(position).read(),
                self.buffer.second.as_ptr().add(position).read(),
            )
        }
    }
}

impl<A, B, L: KeyPiece> Iterator for PairVecIntoIter<A, B, L> {
    type Item = (A, B);

    #[inline]
    fn next(&mut self) -> Option<(A, B)> {
        if self.start == self.buffer.len() {
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
        let len = self.buffer.len() - self.start;
        (len, Some(len))
    }
}

impl<A, B, L: KeyPiece> DoubleEndedIterator for PairVecIntoIter<A, B, L> {
    #[inline]
    fn next_back(&mut self) -> Option<(A, B)> {
        if self.start == self.buffer.len() {
            return None;
        }
        self.buffer.len = self.buffer.len.wrapping_sub(L::ONE);
        // SAFETY: the pair at the new length was not yielded yet, so it is
        // initialized, and lowering the length to it gave up ownership of it.
        Some(unsafe { self.read(self.buffer.len()) })
    }
}

impl<A, B, L: KeyPiece> ExactSizeIterator for PairVecIntoIter<A, B, L> {}
impl<A, B, L: KeyPiece> FusedIterator for PairVecIntoIter<A, B, L> {}

impl<A, B, L: KeyPiece> Drop for PairVecIntoIter<A, B, L> {
    fn drop(&mut self) {
        let remaining = self.buffer.len() - self.start;
        // SAFETY: `start` is never more than the length, which is never more
        // than the capacity, so both pointers stay inside the buffer or one
        // past its end. The pairs from `start` up to the length are
        // initialized and not yielded yet, and nothing uses them after this.
        // `buffer` frees the memory afterwards, even when dropping an item
        // panics.
        unsafe {
            drop_items(
                self.buffer.first.as_ptr().add(self.start),
                self.buffer.second.as_ptr().add(self.start),
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

/// `PairVecRawParts` holds the fields of a [`PairVec`].
/// [`into_raw_parts`](PairVec::into_raw_parts) takes a vec apart into these
/// fields, and [`from_raw_parts`](PairVec::from_raw_parts) builds a vec from
/// them.
///
/// The parts given to `from_raw_parts` must follow every rule below, and the
/// parts that `into_raw_parts` returns always do.
///
/// # Rules
///
/// The layout of the buffer is `Layout::array::<A>(capacity)` extended by
/// `Layout::array::<B>(capacity)` with `Layout::extend`.
///
/// - If that layout takes space, `first` points at a buffer that the global
///   allocator allocated with it, and `second` points at the offset that
///   `Layout::extend` returns, counted in bytes from `first`. Otherwise both
///   pointers only have to be aligned for their items, as dangling pointers
///   are.
/// - `len` is not more than `capacity` or the largest `usize`.
/// - The first `len` items of both slices are initialized.
/// - Nothing else uses, drops or frees the buffer or its items.
///
/// # Examples
///
/// ```
/// use core::alloc::Layout;
/// use core::ptr::NonNull;
/// use gen_map::{PairStorage, PairVec, PairVecRawParts};
///
/// // The buffer has room for two `u32`s followed by two `char`s.
/// let (layout, offset) = Layout::array::<u32>(2)
///     .unwrap()
///     .extend(Layout::array::<char>(2).unwrap())
///     .unwrap();
/// // SAFETY: the layout takes space.
/// let buffer = NonNull::new(unsafe { std::alloc::alloc(layout) }).unwrap();
/// let first = buffer.cast::<u32>();
/// // SAFETY: `offset` is inside the buffer.
/// let second = unsafe { buffer.add(offset) }.cast::<char>();
/// // SAFETY: both pointers point inside the buffer and are aligned for their
/// // items.
/// unsafe {
///     first.write(1);
///     second.write('a');
/// }
/// let parts = PairVecRawParts {
///     first,
///     second,
///     capacity: 2,
///     len: 1,
/// };
/// // SAFETY: the global allocator allocated the buffer with the layout the
/// // rules describe, `second` points at the offset that `Layout::extend`
/// // returned, the first item of each slice is initialized, and nothing else
/// // frees the buffer.
/// let pairs: PairVec<u32, char> = unsafe { PairVec::from_raw_parts(parts) };
/// assert_eq!(pairs.slices(), (&[1][..], &['a'][..]));
/// ```
pub struct PairVecRawParts<A, B, L: KeyPiece = usize> {
    // These fields are in the same order as the fields of the buffer of a
    // `PairVec` on purpose, so that the two structs can be read side by side.
    // `into_raw_parts` and `from_raw_parts` list every field of both structs,
    // so the compiler catches a field that only one of them has, but nothing
    // catches a change in order. Think twice before removing this comment,
    // because it is the only thing that keeps the two orders the same.
    /// `first` points at the first slice, at the start of the buffer.
    pub first: NonNull<A>,
    /// `second` points at the second slice, which starts after the room for
    /// the first slice.
    pub second: NonNull<B>,
    /// `capacity` is how many items each slice has room for.
    pub capacity: L,
    /// `len` is the number of pairs. The first `len` items of each slice are
    /// initialized.
    pub len: L,
}

impl<A, B, L: KeyPiece> PairVec<A, B, L> {
    /// Takes the vec apart into its fields.
    /// [`from_raw_parts`](Self::from_raw_parts) builds a vec from them again,
    /// and [`PairVecRawParts`] lists the rules they follow. Nothing drops the
    /// items or frees the buffer until a vec is built from the parts again.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{PairStorage, PairVec};
    ///
    /// let mut pairs = PairVec::<u32, char, u16>::new();
    /// pairs.push(1, 'a');
    /// pairs.push(2, 'b');
    ///
    /// let mut parts = pairs.into_raw_parts();
    /// assert_eq!(parts.len, 2);
    /// // Leaving out the last pair keeps the parts following the rules.
    /// parts.len = 1;
    /// // SAFETY: the parts came from `into_raw_parts`, and only `len` went
    /// // down.
    /// let pairs = unsafe { PairVec::from_raw_parts(parts) };
    /// assert_eq!(pairs.slices(), (&[1][..], &['a'][..]));
    /// ```
    #[inline]
    pub fn into_raw_parts(self) -> PairVecRawParts<A, B, L> {
        // The vec is never dropped, so its buffer and items now belong to the
        // parts.
        let pairs = ManuallyDrop::new(self);
        let Buffer {
            first,
            second,
            capacity,
            len,
        } = &pairs.buffer;
        PairVecRawParts {
            first: *first,
            second: *second,
            capacity: *capacity,
            len: *len,
        }
    }

    /// Builds a vec from the fields that
    /// [`into_raw_parts`](Self::into_raw_parts) takes a vec apart into. When
    /// the items of both slices take no space, the vec's capacity is the
    /// largest value of the length type, whatever `capacity` the parts hold.
    ///
    /// # Safety
    ///
    /// `parts` must follow every rule listed on [`PairVecRawParts`]. The
    /// vec's methods rely on those rules, and parts that break one can make
    /// them cause undefined behavior.
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_parts(parts: PairVecRawParts<A, B, L>) -> Self {
        let PairVecRawParts {
            first,
            second,
            capacity,
            len,
        } = parts;
        Self {
            buffer: Buffer {
                first,
                second,
                capacity: if takes_no_space::<A, B>() {
                    L::MAX
                } else {
                    capacity
                },
                len,
            },
            _items: PhantomData,
        }
    }
}
