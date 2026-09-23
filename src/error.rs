use core::fmt;

/// Why a [`GenMap`](crate::GenMap) has no room for another value. This is
/// what [`GenMap::vacant_entry`](crate::GenMap::vacant_entry) returns, and
/// what the other insert errors wrap. `E` is the map's
/// [`StorageError`](crate::StorageError).
///
/// When the index type and the storage are both exhausted,
/// [`IndexExhausted`](Self::IndexExhausted) is the one reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullError<E> {
    /// The map has `C::Idx::MAX + 1` slots and none of them are free, so
    /// there is no index left for a new slot.
    IndexExhausted,

    /// None of the slots are free and the storage could not make room for
    /// another one. The field says why, which for a `Vec` is its
    /// `TryReserveError`.
    StorageFull(E),
}

impl<E> FullError<E> {
    /// Returns the same error with the storage's reason borrowed instead of
    /// owned.
    #[inline]
    pub fn as_ref(&self) -> FullError<&E> {
        match self {
            Self::IndexExhausted => FullError::IndexExhausted,
            Self::StorageFull(error) => FullError::StorageFull(error),
        }
    }
}

impl<E: fmt::Display> fmt::Display for FullError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexExhausted => f.write_str("the index type can not address another slot"),
            Self::StorageFull(error) => {
                write!(f, "the storage can not make room for another slot: {error}")
            }
        }
    }
}

/// Why [`GenMap::try_insert`](crate::GenMap::try_insert) could not insert.
/// Each variant hands the value back so that the caller can keep it. `E` is
/// the map's [`StorageError`](crate::StorageError).
///
/// When the index type and the storage are both exhausted,
/// [`IndexExhausted`](Self::IndexExhausted) is the one reported.
pub enum InsertError<T, E> {
    /// The map has `C::Idx::MAX + 1` slots and none of them are free, so
    /// there is no index left for a new slot.
    IndexExhausted(T),

    /// None of the slots are free and the storage could not make room for
    /// another one. The second field says why.
    StorageFull(T, E),
}

impl<T, E> InsertError<T, E> {
    /// Takes the value back out of the error.
    #[inline]
    pub fn into_inner(self) -> T {
        match self {
            Self::IndexExhausted(value) | Self::StorageFull(value, _) => value,
        }
    }

    /// The reason the insert failed, without the value.
    #[inline]
    pub fn kind(&self) -> FullError<&E> {
        match self {
            Self::IndexExhausted(_) => FullError::IndexExhausted,
            Self::StorageFull(_, error) => FullError::StorageFull(error),
        }
    }

    /// Splits the error into its reason and the value.
    #[inline]
    pub fn into_parts(self) -> (FullError<E>, T) {
        match self {
            Self::IndexExhausted(value) => (FullError::IndexExhausted, value),
            Self::StorageFull(value, error) => (FullError::StorageFull(error), value),
        }
    }
}

// Written by hand so that it does not demand `Debug` from `T`.
impl<T, E: fmt::Debug> fmt::Debug for InsertError<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexExhausted(_) => f.write_str("IndexExhausted(..)"),
            Self::StorageFull(_, error) => write!(f, "StorageFull(.., {error:?})"),
        }
    }
}

impl<T, E: fmt::Display> fmt::Display for InsertError<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.kind(), f)
    }
}

/// Why [`GenMap::try_insert_with_key`](crate::GenMap::try_insert_with_key)
/// could not insert. The map can be full before the closure runs, or the
/// closure can refuse to make a value, and this tells the two apart. `E` is
/// the closure's error and `S` is the map's
/// [`StorageError`](crate::StorageError).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsertWithError<E, S> {
    /// The map had no room, so the closure was never called.
    Full(FullError<S>),

    /// Custom error that occurred mid-insert.
    Rejected(E),
}

impl<E, S> InsertWithError<E, S> {
    /// Folds the error into `E` when `E` can represent a full map, so that
    /// a caller with its own error type gets that type back.
    ///
    /// # Examples
    ///
    /// ```
    /// use gen_map::{FullError, GenMap};
    /// use std::collections::TryReserveError;
    ///
    /// #[derive(Debug)]
    /// enum MyError {
    ///     Full(FullError<TryReserveError>),
    ///     Parse,
    /// }
    ///
    /// impl From<FullError<TryReserveError>> for MyError {
    ///     fn from(e: FullError<TryReserveError>) -> Self {
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
        E: From<FullError<S>>,
    {
        match self {
            Self::Full(full) => E::from(full),
            Self::Rejected(error) => error,
        }
    }

    /// Returns the closure's error, or `None` if the map was full.
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

impl<E, S> From<FullError<S>> for InsertWithError<E, S> {
    #[inline]
    fn from(full: FullError<S>) -> Self {
        Self::Full(full)
    }
}

impl<E: fmt::Display, S: fmt::Display> fmt::Display for InsertWithError<E, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(full) => fmt::Display::fmt(full, f),
            Self::Rejected(error) => fmt::Display::fmt(error, f),
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
