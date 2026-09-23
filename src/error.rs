use alloc::collections::TryReserveError;
use core::fmt;

/// Why [`GenMap::try_insert`](crate::GenMap::try_insert) could not insert.
/// Each variant hands the value back so that the caller can keep it.
///
/// When the index type and the storage are both exhausted,
/// [`IndexExhausted`](Self::IndexExhausted) is the one reported.
pub enum InsertError<T> {
    /// The map has `C::Idx::MAX + 1` slots and none of them are free, so
    /// there is no index left for a new slot.
    IndexExhausted(T),

    /// The storage can not hold another slot and none of the existing slots
    /// are free. Only a storage with a fixed capacity, such as
    /// [`ArrayStorage`](crate::ArrayStorage), can report this.
    StorageFull(T),
}

pub enum CustomOrInsertError<T,Custom>{
    Custom(Custom),
    InsertError(InsertError<T>),
}

impl<T> InsertError<T> {
    /// Takes the value back out of the error.
    #[inline]
    pub fn into_inner(self) -> T {
        match self {
            Self::IndexExhausted(value) | Self::StorageFull(value) => value,
        }
    }
}

// Written by hand so that it does not demand `Debug` from `T`.
impl<T> fmt::Debug for InsertError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexExhausted(_) => f.write_str("IndexExhausted(..)"),
            Self::StorageFull(_) => f.write_str("StorageFull(..)"),
        }
    }
}

impl<T> fmt::Display for InsertError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexExhausted(_) => f.write_str("the index type can not address another slot"),
            Self::StorageFull(_) => f.write_str("the storage can not hold another slot"),
        }
    }
}

/// Why [`GenMap::get_disjoint_mut`](crate::GenMap::get_disjoint_mut) could
/// not hand out its references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GetDisjointMutError {
    /// One of the keys is invalid, meaning
    /// [`contains_key`](crate::GenMap::contains_key) returns `false` for it.
    InvalidKey,

    /// Two of the keys point at the same slot, so the references would
    /// alias.
    OverlappingKeys,
}

impl fmt::Display for GetDisjointMutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey => f.write_str("one of the keys is invalid"),
            Self::OverlappingKeys => f.write_str("two of the keys point at the same slot"),
        }
    }
}

/// Why [`GenMap::get_disjoint_mut_at`](crate::GenMap::get_disjoint_mut_at)
/// could not hand out its references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GetDisjointMutAtError {
    /// There is no slot at one of the indices, or the slot holds no value,
    /// meaning [`key_at`](crate::GenMap::key_at) returns `None` for it.
    NoValue,

    /// Two of the indices are the same, so the references would alias.
    OverlappingIndices,
}

impl fmt::Display for GetDisjointMutAtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoValue => f.write_str("one of the indices has no value"),
            Self::OverlappingIndices => f.write_str("two of the indices are the same"),
        }
    }
}

/// Why a [`SlotStorage`](crate::SlotStorage) could not make room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveError {
    /// The storage allocates and the allocation failed. This wraps the
    /// error `Vec::try_reserve` gives.
    Alloc(TryReserveError),

    /// The storage has a fixed capacity and the items would not fit in it.
    CapacityExceeded,
}

impl fmt::Display for ReserveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Alloc(error) => fmt::Display::fmt(error, f),
            Self::CapacityExceeded => {
                f.write_str("the storage has a fixed capacity and the items would not fit in it")
            }
        }
    }
}
