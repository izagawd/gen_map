use core::fmt;

/// Why a [`GenMap`](crate::GenMap) or a [`DenseGenMap`](crate::DenseGenMap)
/// has no room for another value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullError<S> {
    /// The map has a slot at every index it can use and none of them are
    /// free. A map uses every index its keys can hold, except the largest
    /// value of the index type.
    IndexExhausted,

    /// A storage could not make room for the value. For a `GenMap`, that is
    /// the slot storage, and none of the slots are free. The field says why,
    /// which for a `Vec` is its `TryReserveError`.
    StorageFull(S),
}

impl<S> FullError<S> {
    /// Returns the same error with the storage's reason borrowed instead of
    /// owned.
    #[inline]
    pub fn as_ref(&self) -> FullError<&S> {
        match self {
            Self::IndexExhausted => FullError::IndexExhausted,
            Self::StorageFull(error) => FullError::StorageFull(error),
        }
    }
}

impl<S: fmt::Display> fmt::Display for FullError<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexExhausted => f.write_str("the keys cannot address another slot"),
            Self::StorageFull(error) => {
                write!(f, "the storage cannot make room for another slot: {error}")
            }
        }
    }
}

/// Why [`GenMap::try_insert`](crate::GenMap::try_insert) could not insert.
/// Each variant hands the value back so that the caller can keep it. `S` is
/// the map's [`StorageError`](crate::StorageError), or the
/// [`DenseStorageError`](crate::DenseStorageError) of a
/// [`DenseGenMap`](crate::DenseGenMap).
///
/// When the keys have no index left for a new slot and the storage is also
/// full, the error is [`IndexExhausted`](Self::IndexExhausted).
pub enum InsertError<T, S> {
    /// The map has a slot at every index it can use and none of them are
    /// free. A map uses every index its keys can hold, except the largest
    /// value of the index type.
    IndexExhausted(T),

    /// A storage could not make room for the value. For a `GenMap`, that is
    /// the slot storage, and none of the slots are free. The second field says
    /// why.
    StorageFull(T, S),
}

impl<T, S> InsertError<T, S> {
    /// Takes the value back out of the error.
    #[inline]
    pub fn into_inner(self) -> T {
        match self {
            Self::IndexExhausted(value) | Self::StorageFull(value, _) => value,
        }
    }

    /// The reason the insert failed, without the value.
    #[inline]
    pub fn kind(&self) -> FullError<&S> {
        match self {
            Self::IndexExhausted(_) => FullError::IndexExhausted,
            Self::StorageFull(_, error) => FullError::StorageFull(error),
        }
    }

    /// Splits the error into its reason and the value.
    #[inline]
    pub fn into_parts(self) -> (FullError<S>, T) {
        match self {
            Self::IndexExhausted(value) => (FullError::IndexExhausted, value),
            Self::StorageFull(value, error) => (FullError::StorageFull(error), value),
        }
    }
}

// This impl is written by hand so that it does not require `T: Debug`.
impl<T, S: fmt::Debug> fmt::Debug for InsertError<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexExhausted(_) => f.write_str("IndexExhausted(..)"),
            Self::StorageFull(_, error) => write!(f, "StorageFull(.., {error:?})"),
        }
    }
}

impl<T, S: fmt::Display> fmt::Display for InsertError<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.kind(), f)
    }
}

/// Why [`SecondaryMap::insert`](crate::SecondaryMap::insert) could not
/// insert. Each variant hands the value back so that the caller can keep it.
/// `S` is the map's [`SecondaryStorageError`](crate::SecondaryStorageError),
/// or the [`DenseSecondaryStorageError`](crate::DenseSecondaryStorageError)
/// of a [`DenseSecondaryMap`](crate::DenseSecondaryMap).
pub enum SecondaryInsertError<T, S> {
    /// The slot at the key's index holds a value that was inserted under a
    /// different generation, and the config's
    /// [`ReplaceStrategy`](crate::ReplaceStrategy) kept that value.
    Refused(T),

    /// The key's index is the largest value of the index type. No map gives
    /// a slot that index, so only a hand-built key can have it.
    IndexReserved(T),

    /// A storage could not make room for the value. For a `SecondaryMap`, it
    /// is the storage of the slots up to the key's index. The second field says
    /// why.
    StorageFull(T, S),
}

impl<T, S> SecondaryInsertError<T, S> {
    /// Takes the value back out of the error.
    #[inline]
    pub fn into_inner(self) -> T {
        match self {
            Self::Refused(value) | Self::IndexReserved(value) | Self::StorageFull(value, _) => {
                value
            }
        }
    }
}

// This impl is written by hand so that it does not require `T: Debug`.
impl<T, S: fmt::Debug> fmt::Debug for SecondaryInsertError<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(_) => f.write_str("Refused(..)"),
            Self::IndexReserved(_) => f.write_str("IndexReserved(..)"),
            Self::StorageFull(_, error) => write!(f, "StorageFull(.., {error:?})"),
        }
    }
}

impl<T, S: fmt::Display> fmt::Display for SecondaryInsertError<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(_) => f.write_str(
                "the slot holds a value from a different generation, and the replace strategy kept it",
            ),
            Self::IndexReserved(_) => f.write_str(
                "the key's index is the largest value of the index type, which no slot ever has",
            ),
            Self::StorageFull(_, error) => {
                write!(f, "the storage cannot make room for the slot: {error}")
            }
        }
    }
}

/// Why [`GenMap::try_insert_with_key`](crate::GenMap::try_insert_with_key)
/// could not insert. The map can be full before the closure runs, or the
/// closure can refuse to make a value, and the variant says which of the two
/// happened. `E` is the closure's error and `S` is the map's
/// [`StorageError`](crate::StorageError).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InsertWithError<E, S> {
    /// The map had no room, so the closure was never called.
    Full(FullError<S>),

    /// The closure returned this error, so nothing was inserted.
    Rejected(E),
}

impl<E, S> InsertWithError<E, S> {
    /// Converts the error into `E`, using `E`'s `From<FullError<S>>` impl
    /// for a full map, so that a caller with its own error type gets that
    /// type back.
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

/// Why [`GenMap::get_disjoint_mut`](crate::GenMap::get_disjoint_mut) or
/// [`SecondaryMap::get_disjoint_mut`](crate::SecondaryMap::get_disjoint_mut)
/// could not hand out its references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GetDisjointMutError {
    /// The map has no value for one of the keys, meaning the map's
    /// `contains_key` returns `false` for it.
    InvalidKey,

    /// Two or more of the keys point at the same slot, so the references would
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

/// Why [`GenMap::get_disjoint_mut_at`](crate::GenMap::get_disjoint_mut_at) or
/// [`SecondaryMap::get_disjoint_mut_at`](crate::SecondaryMap::get_disjoint_mut_at)
/// could not hand out its references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GetDisjointMutAtError {
    /// There is no slot at one of the indices, or the slot holds no value,
    /// meaning the map's `key_at` returns `None` for it.
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

/// Which storage of a dense map could not make room, together with that
/// storage's error. `S` is the error of the slot storage, `V` is the error of
/// the value storage, and `K` is the error of the key storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenseError<S, V, K> {
    /// The slot storage could not make room for another slot.
    Slots(S),
    /// The value storage could not make room for another value.
    Values(V),
    /// The key storage could not make room for another key.
    Keys(K),
}

impl<S: fmt::Display, V: fmt::Display, K: fmt::Display> fmt::Display for DenseError<S, V, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Slots(error) => write!(f, "the slot storage is full: {error}"),
            Self::Values(error) => write!(f, "the value storage is full: {error}"),
            Self::Keys(error) => write!(f, "the key storage is full: {error}"),
        }
    }
}
