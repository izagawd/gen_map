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

impl<T> InsertError<T> {
    /// Takes the value back out of the error.
    #[inline]
    pub fn into_inner(self) -> T {
        match self {
            Self::IndexExhausted(value) | Self::StorageFull(value) => value,
        }
    }

    /// The reason, without the value.
    #[inline]
    pub fn kind(&self) -> FullError {
        match self {
            Self::IndexExhausted(_) => FullError::IndexExhausted,
            Self::StorageFull(_) => FullError::StorageFull,
        }
    }

    /// Splits the error into its reason and the value.
    #[inline]
    pub fn into_parts(self) -> (FullError, T) {
        match self {
            Self::IndexExhausted(value) => (FullError::IndexExhausted, value),
            Self::StorageFull(value) => (FullError::StorageFull, value),
        }
    }
}

/// When a [`GenMap`](crate::GenMap) has no room for another value, this is
/// what [`GenMap::vacant_entry`](crate::GenMap::vacant_entry) returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullError {
    /// The map has `C::Idx::MAX + 1` slots and none of them are free, so
    /// there is no index left for a new slot.
    IndexExhausted,

    /// The storage can not hold another slot and none of the existing slots
    /// are free. Only a storage with a fixed capacity can report this.
    StorageFull,
}

impl fmt::Display for FullError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexExhausted => f.write_str("the index type can not address another slot"),
            Self::StorageFull => f.write_str("the storage can not hold another slot"),
        }
    }
}

/// Why [`GenMap::try_insert_with_key`](crate::GenMap::try_insert_with_key)
/// could not insert. The map can be full before the closure runs, or the
/// closure can refuse to make a value; this tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsertWithError<E> {
    /// The map had no room. The closure was never called.
    Full(FullError),

    /// Custom error that occurred mid-insert.
    Rejected(E),
}

impl<E> InsertWithError<E> {
    /// Folds the error into `E` when `E` can represent a full map. This
    /// lets a caller with its own error type get that type back:
    ///
    /// ```
    /// use gen_map::{FullError, GenMap};
    ///
    /// #[derive(Debug)]
    /// enum MyError {
    ///     Full(FullError),
    ///     Parse,
    /// }
    ///
    /// impl From<FullError> for MyError {
    ///     fn from(e: FullError) -> Self {
    ///         MyError::Full(e)
    ///     }
    /// }
    ///
    /// fn add(map: &mut GenMap<u32>, text: &str) -> Result<(), MyError> {
    ///     map.try_insert_with_key(|_| text.parse().map_err(|_| MyError::Parse))
    ///         .map_err(|e| e.flatten())?;
    ///     Ok(())
    /// }
    ///
    /// let mut map = GenMap::new();
    /// assert!(add(&mut map, "7").is_ok());
    /// assert!(matches!(add(&mut map, "x"), Err(MyError::Parse)));
    /// ```
    #[inline]
    pub fn flatten(self) -> E
    where
        E: From<FullError>,
    {
        match self {
            Self::Full(full) => E::from(full),
            Self::Rejected(error) => error,
        }
    }

    /// The closure's error, if that is what this is.
    #[inline]
    pub fn rejected(self) -> Option<E> {
        match self {
            Self::Rejected(error) => Some(error),
            Self::Full(_) => None,
        }
    }

    /// Returns `true` if the map was full.
    #[inline]
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }
}

impl<E> From<FullError> for InsertWithError<E> {
    #[inline]
    fn from(full: FullError) -> Self {
        Self::Full(full)
    }
}

impl<E: fmt::Display> fmt::Display for InsertWithError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(full) => fmt::Display::fmt(full, f),
            Self::Rejected(error) => fmt::Display::fmt(error, f),
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
