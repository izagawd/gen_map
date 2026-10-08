use crate::error::ReserveError;
use crate::key::piece::KeyPiece;
use crate::storage::buffer::{allocate, deallocate, grown_capacity, max_len, saturating_usize};
use crate::storage::{ReserveStorage, SliceStorage};
use core::fmt;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::mem::{self, size_of, ManuallyDrop};
use core::ops::{Deref, DerefMut};
use core::ptr::{self, NonNull};
use core::slice;

/// A `SingleVec` keeps its items in one buffer on the heap, where a
/// [`PairVec`](crate::PairVec) keeps them in two. It is a [`SliceStorage`]
/// that works like a `Vec`, but it stores its length and capacity as its
/// length type, the second type parameter. The length type can be `u8`,
/// `u16`, `u32`, `u64`, `u128` or `usize`, which is the default. A
/// `SingleVec` never holds more items than its length type can count, and
/// with a `u32` length type it takes 16 bytes on a 64-bit target, where a
/// `Vec` takes 24.
///
/// With [`DefaultMapConfig`](crate::DefaultMapConfig), every map except a
/// `SparseSecondaryMap` keeps its slots in a `SingleVec` whose length type is
/// the index type of the map's keys. A `SingleVec` needs the `alloc` feature.
///
/// A `SingleVec` derefs to a slice of its items. When its items take no space,
/// it never allocates, and its capacity is the most items its length type can
/// count.
///
/// A `SingleVec` drops its items in its own `Drop` implementation, so the
/// compiler requires everything its items borrow to outlive it.
///
/// # Examples
///
/// ```
/// use gen_map::SingleVec;
///
/// let mut items = SingleVec::<&str, u32>::new();
/// items.push("a");
/// items.push("b");
/// assert_eq!(items[..], ["a", "b"]);
/// assert_eq!(items.len(), 2);
/// ```
pub struct SingleVec<T, L: KeyPiece = usize> {
    buffer: Buffer<T, L>,
    /// A `SingleVec` owns the items in its buffer.
    _items: PhantomData<T>,
}

/// The buffer of a [`SingleVec`] or a [`SingleVecIntoIter`], together with its
/// capacity and how many of its items are initialized. Dropping a `Buffer`
/// frees the memory without dropping any item in it.
struct Buffer<T, L: KeyPiece> {
    pointer: NonNull<T>,
    /// How many items the buffer has room for. When the items take no space,
    /// it is the largest value of the length type.
    capacity: L,
    /// The number of items. The first `len` items of the buffer are
    /// initialized, and the rest are not. It is never more than the capacity
    /// or the largest `usize`.
    len: L,
}

impl<T, L: KeyPiece> Buffer<T, L> {
    /// Returns a buffer that holds no memory. Its capacity is zero, or the
    /// largest value of the length type when the items take no space.
    #[inline]
    const fn new() -> Self {
        Self {
            pointer: NonNull::dangling(),
            capacity: if size_of::<T>() == 0 { L::MAX } else { L::ZERO },
            len: L::ZERO,
        }
    }

    /// Allocates a buffer with room for `capacity` items.
    fn with_capacity(capacity: usize) -> Result<Self, ReserveError> {
        let stored = L::from_usize(capacity).ok_or(ReserveError::CapacityOverflow)?;
        if size_of::<T>() == 0 {
            return Ok(Self::new());
        }
        Ok(Self {
            pointer: allocate(capacity)?,
            capacity: stored,
            len: L::ZERO,
        })
    }

    /// The number of items as a `usize`.
    #[inline]
    fn len(&self) -> usize {
        saturating_usize(self.len)
    }

    /// How many items the buffer has room for, as a `usize`. A capacity
    /// larger than the largest `usize` counts as the largest `usize`.
    #[inline]
    fn capacity(&self) -> usize {
        saturating_usize(self.capacity)
    }
}

impl<T, L: KeyPiece> Drop for Buffer<T, L> {
    fn drop(&mut self) {
        // SAFETY: unless `capacity` items take no space, the buffer was
        // allocated with the layout `deallocate` expects, by `allocate` or as
        // the rules of `SingleVecRawParts` require. Nothing uses it after
        // this.
        unsafe { deallocate(self.pointer, self.capacity()) };
    }
}

impl<T, L: KeyPiece> SingleVec<T, L> {
    /// Creates a `SingleVec` with no items. It allocates nothing until an item
    /// is pushed.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            _items: PhantomData,
        }
    }

    /// Creates a `SingleVec` with no items and room for at least `capacity` of
    /// them.
    ///
    /// # Panics
    ///
    /// Panics if the length type cannot count `capacity` items, or if a
    /// buffer with room for them would take more than `isize::MAX` bytes. If
    /// the allocation fails, `with_capacity` calls `handle_alloc_error`, which
    /// aborts the program by default.
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

    /// Appends `item`.
    ///
    /// # Panics
    ///
    /// Panics if the `SingleVec` would hold more items than its length type can
    /// count, or need a buffer that takes more than `isize::MAX` bytes. If an
    /// allocation fails, `push` calls `handle_alloc_error`, which aborts the
    /// program by default.
    #[inline]
    pub fn push(&mut self, item: T) {
        if let Err(error) = self.make_room(1) {
            error.handle();
        }
        // SAFETY: `make_room` just made room for one more item.
        unsafe { self.push_unchecked(item) };
    }

    /// Appends `item`.
    ///
    /// # Errors
    ///
    /// Hands `item` back if the `SingleVec` cannot make room for it.
    #[inline]
    pub fn try_push(&mut self, item: T) -> Result<(), T> {
        if self.make_room(1).is_err() {
            return Err(item);
        }
        // SAFETY: `make_room` just made room for one more item.
        unsafe { self.push_unchecked(item) };
        Ok(())
    }

    /// Makes room for at least `additional` more items. When the buffer is
    /// too small, it moves the items into a new buffer with at least twice
    /// the old capacity, unless the length type cannot count that many items.
    fn make_room(&mut self, additional: usize) -> Result<(), ReserveError> {
        let len = self.buffer.len();
        let required = len
            .checked_add(additional)
            .ok_or(ReserveError::CapacityOverflow)?;
        if required <= self.buffer.capacity() {
            return Ok(());
        }
        let capacity = grown_capacity(
            self.buffer.capacity(),
            required,
            max_len::<L>(),
            size_of::<T>(),
        )?;
        let mut buffer = Buffer::with_capacity(capacity)?;
        // SAFETY: the new buffer has room for more than `len` items, and it
        // does not overlap the old one. The items move into the new buffer,
        // and the old buffer is freed without dropping them.
        unsafe {
            ptr::copy_nonoverlapping(self.buffer.pointer.as_ptr(), buffer.pointer.as_ptr(), len);
        }
        buffer.len = self.buffer.len;
        mem::swap(&mut self.buffer, &mut buffer);
        Ok(())
    }

    /// Appends `item` without checking that there is room for it.
    ///
    /// # Safety
    ///
    /// The length must be below the capacity.
    #[inline]
    unsafe fn push_unchecked(&mut self, item: T) {
        let len = self.buffer.len();
        debug_assert!(len < self.buffer.capacity());
        // SAFETY: the caller promises that the length is below the capacity,
        // so the buffer has room for an item at position `len`.
        unsafe { self.buffer.pointer.as_ptr().add(len).write(item) };
        // The length is below the capacity, which is a value of the length
        // type, so adding one does not wrap.
        self.buffer.len = self.buffer.len.wrapping_add(L::ONE);
    }
}

impl<T, L: KeyPiece> Default for SingleVec<T, L> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T, L: KeyPiece> Drop for SingleVec<T, L> {
    fn drop(&mut self) {
        let items = ptr::slice_from_raw_parts_mut(self.buffer.pointer.as_ptr(), self.buffer.len());
        // SAFETY: the first `len` items of the buffer are initialized, and
        // nothing uses them after this. `buffer` frees the memory afterwards,
        // even when dropping an item panics.
        unsafe { ptr::drop_in_place(items) };
    }
}

impl<T, L: KeyPiece> Deref for SingleVec<T, L> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        // SAFETY: the first `len` items of the buffer are initialized.
        unsafe { slice::from_raw_parts(self.buffer.pointer.as_ptr(), self.buffer.len()) }
    }
}

impl<T, L: KeyPiece> DerefMut for SingleVec<T, L> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: the first `len` items of the buffer are initialized, and
        // `&mut self` keeps any other reference to them from existing.
        unsafe { slice::from_raw_parts_mut(self.buffer.pointer.as_ptr(), self.buffer.len()) }
    }
}

// SAFETY: the first `len` items of the buffer are initialized and are the
// items pushed since the last `clear` and not popped, in the order they were
// pushed. `try_push` writes an item only once there is room for it, `pop` and
// `clear` lower the length before they move or drop an item, and growing
// copies the items into the new buffer without changing them. Once
// `ensure_room` returns `Ok` for `n` items, the capacity is at least the
// length plus `n`, so the next `n` pushes fit without growing. Only
// `with_capacity` panics on its own, and `clear` only panics when dropping an
// item does.
unsafe impl<T, L: KeyPiece> SliceStorage for SingleVec<T, L> {
    type Item = T;
    type Error = ReserveError;

    #[inline]
    fn empty() -> Self {
        Self::new()
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        SingleVec::with_capacity(capacity)
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    #[inline]
    fn as_slice(&self) -> &[T] {
        self
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [T] {
        self
    }

    #[inline]
    fn ensure_room(&mut self, additional: usize) -> Result<(), ReserveError> {
        self.make_room(additional)
    }

    #[inline]
    fn try_push(&mut self, item: T) -> Result<(), T> {
        SingleVec::try_push(self, item)
    }

    #[inline]
    fn pop(&mut self) -> Option<T> {
        if self.buffer.len == L::ZERO {
            return None;
        }
        self.buffer.len = self.buffer.len.wrapping_sub(L::ONE);
        // SAFETY: the item at the new length was the initialized last item,
        // and lowering the length gave up ownership of it.
        Some(unsafe { self.buffer.pointer.as_ptr().add(self.buffer.len()).read() })
    }

    #[inline]
    fn clear(&mut self) {
        let items = ptr::slice_from_raw_parts_mut(self.buffer.pointer.as_ptr(), self.buffer.len());
        self.buffer.len = L::ZERO;
        // SAFETY: the items were initialized, and setting the length to zero
        // gave up ownership of them, so the storage is empty even when
        // dropping an item panics.
        unsafe { ptr::drop_in_place(items) };
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

impl<T, L: KeyPiece> ReserveStorage for SingleVec<T, L> {}

// SAFETY: a `SingleVec` owns its items the way a `Vec` does, so it can move to
// another thread when its items can.
unsafe impl<T: Send, L: KeyPiece> Send for SingleVec<T, L> {}

// SAFETY: a shared `SingleVec` only hands out shared references to its items,
// so it can be shared between threads when its items can.
unsafe impl<T: Sync, L: KeyPiece> Sync for SingleVec<T, L> {}

impl<T: Clone, L: KeyPiece> Clone for SingleVec<T, L> {
    fn clone(&self) -> Self {
        let mut clone = Self::with_capacity(self.buffer.len());
        for item in self.iter() {
            clone.push(item.clone());
        }
        clone
    }
}

impl<T: fmt::Debug, L: KeyPiece> fmt::Debug for SingleVec<T, L> {
    /// Lists the items in the order they were pushed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T, L: KeyPiece> FromIterator<T> for SingleVec<T, L> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut items = Self::with_capacity(iter.size_hint().0);
        for item in iter {
            items.push(item);
        }
        items
    }
}

impl<T, L: KeyPiece> IntoIterator for SingleVec<T, L> {
    type Item = T;
    type IntoIter = SingleVecIntoIter<T, L>;

    #[inline]
    fn into_iter(self) -> SingleVecIntoIter<T, L> {
        let items = ManuallyDrop::new(self);
        // SAFETY: `items` is never dropped, so the buffer moves into the
        // iterator and is only freed once, by the iterator.
        let buffer = unsafe { ptr::read(&items.buffer) };
        SingleVecIntoIter {
            buffer,
            start: 0,
            _items: PhantomData,
        }
    }
}

/// Owning iterator over the items of a [`SingleVec`], in the order they were
/// pushed. It is created by consuming a `SingleVec` with `into_iter`. Dropping
/// it drops the items it has not yielded yet.
pub struct SingleVecIntoIter<T, L: KeyPiece = usize> {
    /// The buffer of the `SingleVec`. Its length is one past the position of
    /// the item that `next_back` yields.
    buffer: Buffer<T, L>,
    /// The position of the item that `next` yields. The items from `start` up
    /// to the length of the buffer are initialized and not yielded yet.
    start: usize,
    /// The iterator owns the items it has not yielded yet.
    _items: PhantomData<T>,
}

impl<T, L: KeyPiece> Iterator for SingleVecIntoIter<T, L> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        if self.start == self.buffer.len() {
            return None;
        }
        let position = self.start;
        self.start += 1;
        // SAFETY: the item at `position` was not yielded yet, so it is
        // initialized, and raising `start` past it gave up ownership of it.
        Some(unsafe { self.buffer.pointer.as_ptr().add(position).read() })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.buffer.len() - self.start;
        (len, Some(len))
    }
}

impl<T, L: KeyPiece> DoubleEndedIterator for SingleVecIntoIter<T, L> {
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        if self.start == self.buffer.len() {
            return None;
        }
        self.buffer.len = self.buffer.len.wrapping_sub(L::ONE);
        // SAFETY: the item at the new length was not yielded yet, so it is
        // initialized, and lowering the length to it gave up ownership of it.
        Some(unsafe { self.buffer.pointer.as_ptr().add(self.buffer.len()).read() })
    }
}

impl<T, L: KeyPiece> ExactSizeIterator for SingleVecIntoIter<T, L> {}
impl<T, L: KeyPiece> FusedIterator for SingleVecIntoIter<T, L> {}

impl<T, L: KeyPiece> Drop for SingleVecIntoIter<T, L> {
    fn drop(&mut self) {
        let remaining = self.buffer.len() - self.start;
        // SAFETY: `start` is never more than the length, which is never more
        // than the capacity, so the pointer stays inside the buffer or one
        // past its end.
        let first = unsafe { self.buffer.pointer.as_ptr().add(self.start) };
        let items = ptr::slice_from_raw_parts_mut(first, remaining);
        // SAFETY: the items from `start` up to the length are initialized and
        // not yielded yet, and nothing uses them after this. `buffer` frees
        // the memory afterwards, even when dropping an item panics.
        unsafe { ptr::drop_in_place(items) };
    }
}

// SAFETY: the iterator owns the items it has not yielded yet, the way a
// `SingleVec` owns its items.
unsafe impl<T: Send, L: KeyPiece> Send for SingleVecIntoIter<T, L> {}

// SAFETY: a shared iterator hands out no references to its items.
unsafe impl<T: Sync, L: KeyPiece> Sync for SingleVecIntoIter<T, L> {}

/// `SingleVecRawParts` holds the fields of a [`SingleVec`].
/// [`into_raw_parts`](SingleVec::into_raw_parts) takes a vec apart into these
/// fields, and [`from_raw_parts`](SingleVec::from_raw_parts) builds a vec from
/// them.
///
/// The parts given to `from_raw_parts` must follow every rule below, and the
/// parts that `into_raw_parts` returns always do.
///
/// # Rules
///
/// - If the items take space and `capacity` is above zero, `pointer` points
///   at a buffer that the global allocator allocated with the layout that
///   `Layout::array` gives for `capacity` items. Otherwise `pointer` only has
///   to be aligned for the items, as a dangling pointer is.
/// - `len` is not more than `capacity` or the largest `usize`.
/// - The first `len` items of the buffer are initialized.
/// - Nothing else uses, drops or frees the buffer or its items.
///
/// # Examples
///
/// ```
/// use core::mem::ManuallyDrop;
/// use core::ptr::NonNull;
/// use gen_map::{SingleVec, SingleVecRawParts};
///
/// let mut vec = ManuallyDrop::new(vec![1, 2, 3]);
/// let parts = SingleVecRawParts {
///     pointer: NonNull::new(vec.as_mut_ptr()).unwrap(),
///     capacity: u32::try_from(vec.capacity()).unwrap(),
///     len: u32::try_from(vec.len()).unwrap(),
/// };
/// // SAFETY: the global allocator allocated the buffer of the `Vec` for its
/// // capacity, its first three items are initialized, and the `Vec` is never
/// // dropped, so nothing else frees the buffer.
/// let items: SingleVec<i32, u32> = unsafe { SingleVec::from_raw_parts(parts) };
/// assert_eq!(items[..], [1, 2, 3]);
/// ```
pub struct SingleVecRawParts<T, L: KeyPiece = usize> {
    // These fields are in the same order as the fields of the buffer of a
    // `SingleVec` on purpose, so that the two structs can be read side by
    // side. `into_raw_parts` and `from_raw_parts` list every field of both
    // structs, so the compiler catches a field that only one of them has, but
    // nothing catches a change in order. Think twice before removing this
    // comment, because it is the only thing that keeps the two orders the
    // same.
    /// `pointer` points at the buffer, which has room for `capacity` items.
    pub pointer: NonNull<T>,
    /// `capacity` is how many items the buffer has room for.
    pub capacity: L,
    /// `len` is the number of items. The first `len` items of the buffer are
    /// initialized.
    pub len: L,
}

impl<T, L: KeyPiece> SingleVec<T, L> {
    /// Takes the vec apart into its fields.
    /// [`from_raw_parts`](Self::from_raw_parts) builds a vec from them again,
    /// and [`SingleVecRawParts`] lists the rules they follow. Nothing drops
    /// the items or frees the buffer until a vec is built from the parts
    /// again.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::SingleVec;
    ///
    /// let mut items = SingleVec::<u32, u16>::new();
    /// items.push(1);
    /// items.push(2);
    ///
    /// let mut parts = items.into_raw_parts();
    /// assert_eq!(parts.len, 2);
    /// // Leaving out the last item keeps the parts following the rules.
    /// parts.len = 1;
    /// // SAFETY: the parts came from `into_raw_parts`, and only `len` went
    /// // down.
    /// let items = unsafe { SingleVec::from_raw_parts(parts) };
    /// assert_eq!(items[..], [1]);
    /// ```
    #[inline]
    pub fn into_raw_parts(self) -> SingleVecRawParts<T, L> {
        // The vec is never dropped, so its buffer and items now belong to the
        // parts.
        let items = ManuallyDrop::new(self);
        let Buffer {
            pointer,
            capacity,
            len,
        } = &items.buffer;
        SingleVecRawParts {
            pointer: *pointer,
            capacity: *capacity,
            len: *len,
        }
    }

    /// Builds a vec from the fields that
    /// [`into_raw_parts`](Self::into_raw_parts) takes a vec apart into. When
    /// the items take no space, the vec's capacity is the largest value of the
    /// length type, whatever `capacity` the parts hold.
    ///
    /// # Safety
    ///
    /// `parts` must follow every rule listed on [`SingleVecRawParts`]. The
    /// vec's methods rely on those rules, and parts that break one can make
    /// them cause undefined behavior.
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_parts(parts: SingleVecRawParts<T, L>) -> Self {
        let SingleVecRawParts {
            pointer,
            capacity,
            len,
        } = parts;
        Self {
            buffer: Buffer {
                pointer,
                capacity: if size_of::<T>() == 0 {
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
