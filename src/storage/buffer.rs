use crate::error::ReserveError;
use crate::key::piece::KeyPiece;
use alloc::alloc::{Allocator, Layout};
use core::ptr::NonNull;

/// Returns `value` as a `usize`, or the largest `usize` if it is larger.
#[inline]
pub(crate) fn saturating_usize<L: KeyPiece>(value: L) -> usize {
    value.into_usize().unwrap_or(usize::MAX)
}

/// Returns the most items a vec can hold when it stores its length and
/// capacity in this length type. That is the largest value of the length
/// type, or the largest `usize` if that is smaller.
#[inline]
pub(crate) fn max_len<L: KeyPiece>() -> usize {
    saturating_usize(L::MAX)
}

/// Writes `count` into the length it borrows when it is dropped.
pub(crate) struct SetLenOnDrop<'a, L: KeyPiece> {
    len: &'a mut L,
    /// `count` is the number of items or pairs written so far.
    pub(crate) count: L,
}

impl<'a, L: KeyPiece> SetLenOnDrop<'a, L> {
    /// Returns a `SetLenOnDrop` for `len` whose count starts at zero.
    #[inline]
    pub(crate) fn new(len: &'a mut L) -> Self {
        Self {
            len,
            count: L::ZERO,
        }
    }
}

impl<L: KeyPiece> Drop for SetLenOnDrop<'_, L> {
    #[inline]
    fn drop(&mut self) {
        *self.len = self.count;
    }
}

/// Returns the capacity that a vec with room for `capacity` items grows to
/// when it needs room for `required` items. The new capacity is at least
/// twice the old one, but it is never more than `max`.
///
/// # Errors
///
/// Returns [`ReserveError::CapacityOverflow`] if `required` is more than
/// `max`.
pub(crate) fn grown_capacity(
    capacity: usize,
    required: usize,
    max: usize,
    item_size: usize,
) -> Result<usize, ReserveError> {
    if required > max {
        return Err(ReserveError::CapacityOverflow);
    }
    // A new buffer has room for at least four items when they are small,
    // which saves allocations, and for at least one item when they are large,
    // which saves memory.
    let smallest = if item_size <= 1024 { 4 } else { 1 };
    Ok(required
        .max(capacity.saturating_mul(2))
        .max(smallest)
        .min(max))
}

/// Allocates a buffer with the layout `layout` from `allocator`. `layout`
/// must take space, because [`deallocate_layout`] does not give a buffer
/// whose layout takes no space back to the allocator.
///
/// # Errors
///
/// Returns [`ReserveError::AllocError`] if the allocator cannot allocate the
/// buffer.
pub(crate) fn allocate_layout<A: Allocator>(
    allocator: &A,
    layout: Layout,
) -> Result<NonNull<u8>, ReserveError> {
    debug_assert!(layout.size() != 0);
    allocator
        .allocate(layout)
        .map(NonNull::cast)
        .map_err(|_| ReserveError::AllocError(layout))
}

/// Grows a buffer with the layout `old` into a buffer with the layout `new`,
/// and returns the new buffer. The new buffer starts with the bytes of the
/// old one. When `old` takes no space, the old buffer holds no memory, so
/// this allocates the new buffer instead.
///
/// # Errors
///
/// Returns [`ReserveError::AllocError`] if the allocator cannot make the new
/// buffer. The old buffer is left as it was.
///
/// # Safety
///
/// If `old` takes space, `allocator` must have allocated `pointer` with the
/// layout `old`. `new` must have the alignment of `old` and take more space
/// than `old`. Once this returns `Ok`, nothing may use the old buffer.
pub(crate) unsafe fn reallocate_layout<A: Allocator>(
    allocator: &A,
    pointer: NonNull<u8>,
    old: Layout,
    new: Layout,
) -> Result<NonNull<u8>, ReserveError> {
    debug_assert_eq!(old.align(), new.align());
    debug_assert!(new.size() > old.size());
    if old.size() == 0 {
        // `new` takes more space than `old`, so it takes space.
        return allocate_layout(allocator, new);
    }
    // SAFETY: the caller promises that `allocator` allocated `pointer` with
    // `old`, and that `new` takes more space than `old`.
    unsafe { allocator.grow(pointer, old, new) }
        .map(NonNull::cast)
        .map_err(|_| ReserveError::AllocError(new))
}

/// Frees a buffer with the layout `layout`. A buffer whose layout takes no
/// space holds no memory, so it is left as it is.
///
/// # Safety
///
/// If `layout` takes space, `allocator` must have allocated `pointer` with
/// it. Nothing may use the buffer afterwards.
pub(crate) unsafe fn deallocate_layout<A: Allocator>(
    allocator: &A,
    pointer: NonNull<u8>,
    layout: Layout,
) {
    if layout.size() != 0 {
        // SAFETY: the layout takes space, so the caller promises that
        // `allocator` allocated `pointer` with it and that nothing uses the
        // buffer afterwards.
        unsafe { allocator.deallocate(pointer, layout) };
    }
}

/// Allocates a buffer with room for `capacity` items from `allocator`. A
/// buffer for items that take no space, or for no items, needs no memory, so
/// it is a dangling pointer.
pub(crate) fn allocate<T, A: Allocator>(
    allocator: &A,
    capacity: usize,
) -> Result<NonNull<T>, ReserveError> {
    let layout = Layout::array::<T>(capacity).map_err(|_| ReserveError::CapacityOverflow)?;
    if layout.size() == 0 {
        return Ok(NonNull::dangling());
    }
    allocate_layout(allocator, layout).map(NonNull::cast)
}

/// Grows a buffer with room for `old_capacity` items into one with room for
/// `new_capacity` items, as [`reallocate_layout`] does, and returns it. A
/// buffer for items that take no space needs no memory, so it stays a
/// dangling pointer.
///
/// # Errors
///
/// Returns [`ReserveError::CapacityOverflow`] if the new buffer would take
/// more than `isize::MAX` bytes, and [`ReserveError::AllocError`] if the
/// allocator cannot make it. The old buffer is left as it was in both cases.
///
/// # Safety
///
/// Unless `old_capacity` items take no space, `allocator` must have allocated
/// `pointer` with the layout that `Layout::array` gives for `old_capacity`
/// items. `new_capacity` must be more than `old_capacity`. Once this returns
/// `Ok`, nothing may use the old buffer.
pub(crate) unsafe fn reallocate<T, A: Allocator>(
    allocator: &A,
    pointer: NonNull<T>,
    old_capacity: usize,
    new_capacity: usize,
) -> Result<NonNull<T>, ReserveError> {
    debug_assert!(new_capacity > old_capacity);
    let new = Layout::array::<T>(new_capacity).map_err(|_| ReserveError::CapacityOverflow)?;
    if new.size() == 0 {
        return Ok(NonNull::dangling());
    }
    // SAFETY: the caller promises that the buffer was allocated with this
    // layout, or that `old_capacity` items take no space, so making the layout
    // again succeeds.
    let old = unsafe { Layout::array::<T>(old_capacity).unwrap_unchecked() };
    // SAFETY: if `old` takes space, the caller promises that `allocator`
    // allocated `pointer` with it. Both layouts have the alignment of `T`. The
    // check above shows that the items take space, and `new` has room for more
    // of them, so `new` takes more space than `old`.
    unsafe { reallocate_layout(allocator, pointer.cast(), old, new) }.map(NonNull::cast)
}

/// Frees a buffer that `allocator` allocated for `capacity` items, as
/// [`allocate`] and [`reallocate`] do. A buffer that holds no memory is left
/// as it is.
///
/// # Safety
///
/// Unless `capacity` items take no space, `allocator` must have allocated
/// `pointer` with the layout that `Layout::array` gives for `capacity` items.
/// Nothing may use the buffer afterwards.
pub(crate) unsafe fn deallocate<T, A: Allocator>(
    allocator: &A,
    pointer: NonNull<T>,
    capacity: usize,
) {
    // SAFETY: the caller promises that the buffer was allocated with this
    // layout, or that `capacity` items take no space, so making the layout
    // again succeeds.
    let layout = unsafe { Layout::array::<T>(capacity).unwrap_unchecked() };
    // SAFETY: if the layout takes space, the caller promises that `allocator`
    // allocated `pointer` with it and that nothing uses the buffer afterwards.
    unsafe { deallocate_layout(allocator, pointer.cast(), layout) };
}
