use crate::buffer::{allocate, deallocate, grown_capacity, max_len, saturating_usize};
use crate::error::ReserveError;
use crate::key_piece::KeyPiece;
use crate::storage::{ReserveStorage, SliceStorage};
use core::fmt;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::mem::{self, size_of, ManuallyDrop};
use core::ops::{Deref, DerefMut};
use core::ptr::{self, NonNull};
use core::slice;

/// A [`SliceStorage`] that keeps its items in a buffer on the heap, like a
/// `Vec`, but stores its length and capacity as its length type, the second
/// type parameter. The length type can be `u8`, `u16`, `u32`, `u64`, `u128`
/// or `usize`, which is the default. A `LenVec` never holds more items than
/// its length type can count, and with a `u32` length type it takes 16 bytes
/// on a 64-bit target, where a `Vec` takes 24.
///
/// [`DefaultMapConfig`](crate::DefaultMapConfig) keeps the slots of every map
/// in a `LenVec` whose length type is the index type of the map's keys. A
/// `LenVec` needs the `alloc` feature.
///
/// A `LenVec` derefs to a slice of its items. When its items take no space,
/// it never allocates, and its capacity is the most items its length type can
/// count.
///
/// A `LenVec` drops its items in its own `Drop` implementation, so the
/// compiler requires everything its items borrow to outlive it.
///
/// # Examples
///
/// ```
/// use gen_map::LenVec;
///
/// let mut items = LenVec::<&str, u32>::new();
/// items.push("a");
/// items.push("b");
/// assert_eq!(items[..], ["a", "b"]);
/// assert_eq!(items.len(), 2);
/// ```
pub struct LenVec<T, L: KeyPiece = usize> {
    buffer: Buffer<T, L>,
    /// A `LenVec` owns the items in its buffer.
    _items: PhantomData<T>,
}

/// The buffer of a [`LenVec`] or a [`LenVecIntoIter`], together with its
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
        // SAFETY: the buffer either came from `allocate` for `capacity`
        // items, or is for `capacity` items that take no space. Nothing uses
        // it after this.
        unsafe { deallocate(self.pointer, self.capacity()) };
    }
}

impl<T, L: KeyPiece> LenVec<T, L> {
    /// Creates a `LenVec` with no items. It allocates nothing until an item
    /// is pushed.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            _items: PhantomData,
        }
    }

    /// Creates a `LenVec` with no items and room for at least `capacity` of
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
    /// Panics if the `LenVec` would hold more items than its length type can
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
    /// Hands `item` back if the `LenVec` cannot make room for it.
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
    /// the old capacity.
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

impl<T, L: KeyPiece> Default for LenVec<T, L> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T, L: KeyPiece> Drop for LenVec<T, L> {
    fn drop(&mut self) {
        let items = ptr::slice_from_raw_parts_mut(self.buffer.pointer.as_ptr(), self.buffer.len());
        // SAFETY: the first `len` items of the buffer are initialized, and
        // nothing uses them after this. `buffer` frees the memory afterwards,
        // even when dropping an item panics.
        unsafe { ptr::drop_in_place(items) };
    }
}

impl<T, L: KeyPiece> Deref for LenVec<T, L> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        // SAFETY: the first `len` items of the buffer are initialized.
        unsafe { slice::from_raw_parts(self.buffer.pointer.as_ptr(), self.buffer.len()) }
    }
}

impl<T, L: KeyPiece> DerefMut for LenVec<T, L> {
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
// length plus `n`, so the next `n` pushes fit without growing.
unsafe impl<T, L: KeyPiece> SliceStorage for LenVec<T, L> {
    type Item = T;
    type Error = ReserveError;

    #[inline]
    fn empty() -> Self {
        Self::new()
    }

    #[inline]
    fn with_capacity(capacity: usize) -> Self {
        LenVec::with_capacity(capacity)
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
        LenVec::try_push(self, item)
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

impl<T, L: KeyPiece> ReserveStorage for LenVec<T, L> {}

// SAFETY: a `LenVec` owns its items the way a `Vec` does, so it can move to
// another thread when its items can.
unsafe impl<T: Send, L: KeyPiece> Send for LenVec<T, L> {}

// SAFETY: a shared `LenVec` only hands out shared references to its items, so
// it can be shared between threads when its items can.
unsafe impl<T: Sync, L: KeyPiece> Sync for LenVec<T, L> {}

impl<T: Clone, L: KeyPiece> Clone for LenVec<T, L> {
    fn clone(&self) -> Self {
        let mut clone = Self::with_capacity(self.buffer.len());
        for item in self.iter() {
            clone.push(item.clone());
        }
        clone
    }
}

impl<T: fmt::Debug, L: KeyPiece> fmt::Debug for LenVec<T, L> {
    /// Lists the items in the order they were pushed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T, L: KeyPiece> FromIterator<T> for LenVec<T, L> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut items = Self::with_capacity(iter.size_hint().0);
        for item in iter {
            items.push(item);
        }
        items
    }
}

impl<T, L: KeyPiece> IntoIterator for LenVec<T, L> {
    type Item = T;
    type IntoIter = LenVecIntoIter<T, L>;

    #[inline]
    fn into_iter(self) -> LenVecIntoIter<T, L> {
        let items = ManuallyDrop::new(self);
        // SAFETY: `items` is never dropped, so the buffer moves into the
        // iterator and is only freed once, by the iterator.
        let buffer = unsafe { ptr::read(&items.buffer) };
        LenVecIntoIter {
            buffer,
            start: 0,
            _items: PhantomData,
        }
    }
}

/// Owning iterator over the items of a [`LenVec`], in the order they were
/// pushed. It is created by consuming a `LenVec` with `into_iter`. Dropping
/// it drops the items it has not yielded yet.
pub struct LenVecIntoIter<T, L: KeyPiece = usize> {
    /// The buffer of the `LenVec`. Its length is one past the position of the
    /// item that `next_back` yields.
    buffer: Buffer<T, L>,
    /// The position of the item that `next` yields. The items from `start` up
    /// to the length of the buffer are initialized and not yielded yet.
    start: usize,
    /// The iterator owns the items it has not yielded yet.
    _items: PhantomData<T>,
}

impl<T, L: KeyPiece> Iterator for LenVecIntoIter<T, L> {
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

impl<T, L: KeyPiece> DoubleEndedIterator for LenVecIntoIter<T, L> {
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

impl<T, L: KeyPiece> ExactSizeIterator for LenVecIntoIter<T, L> {}
impl<T, L: KeyPiece> FusedIterator for LenVecIntoIter<T, L> {}

impl<T, L: KeyPiece> Drop for LenVecIntoIter<T, L> {
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
// `LenVec` owns its items.
unsafe impl<T: Send, L: KeyPiece> Send for LenVecIntoIter<T, L> {}

// SAFETY: a shared iterator hands out no references to its items.
unsafe impl<T: Sync, L: KeyPiece> Sync for LenVecIntoIter<T, L> {}
