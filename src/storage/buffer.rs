use crate::error::ReserveError;
use crate::key::piece::KeyPiece;
use alloc::alloc::{alloc, dealloc, Layout};
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

/// Returns the capacity that a vec with room for `capacity` items grows to
/// when it needs room for `required` items. The new capacity is at least
/// twice the old one, so a long run of pushes allocates only a few times, but
/// it is never more than `max`.
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

/// Allocates a buffer with room for `capacity` items. A buffer for items that
/// take no space, or for no items, needs no memory, so it is a dangling
/// pointer.
pub(crate) fn allocate<T>(capacity: usize) -> Result<NonNull<T>, ReserveError> {
    let layout = Layout::array::<T>(capacity).map_err(|_| ReserveError::CapacityOverflow)?;
    if layout.size() == 0 {
        return Ok(NonNull::dangling());
    }
    // SAFETY: the layout's size is above zero.
    let pointer = unsafe { alloc(layout) };
    NonNull::new(pointer.cast::<T>()).ok_or(ReserveError::AllocError(layout))
}

/// Frees a buffer that [`allocate`] returned for `capacity` items. A buffer
/// that holds no memory is left as it is.
///
/// # Safety
///
/// `pointer` must have come from `allocate` for `capacity` items, or the
/// buffer must be for `capacity` items that take no space. Nothing may use
/// the buffer afterwards.
pub(crate) unsafe fn deallocate<T>(pointer: NonNull<T>, capacity: usize) {
    // SAFETY: `allocate` made this layout for `capacity` items, or `capacity`
    // items take no space, so making the layout again succeeds.
    let layout = unsafe { Layout::array::<T>(capacity).unwrap_unchecked() };
    if layout.size() != 0 {
        // SAFETY: the layout takes space, so `allocate` allocated `pointer`
        // with this layout, and the caller promises that nothing uses the
        // buffer afterwards.
        unsafe { dealloc(pointer.as_ptr().cast(), layout) };
    }
}
