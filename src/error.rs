use crate::config::KeyConfig;
use crate::key::Key;
#[cfg(feature = "alloc")]
use alloc::alloc::handle_alloc_error;
#[cfg(feature = "alloc")]
use core::alloc::Layout;
use core::fmt;

/// This error says why a [`GenMap`](crate::GenMap) or a
/// [`DenseGenMap`](crate::DenseGenMap) has no room for another value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FullError<S> {
    /// The map has a slot at every index it can use and none of them are
    /// free. A map uses every index its keys can hold, except the largest
    /// value of the index type.
    IndexExhausted,

    /// A storage could not make room for the value. For a `GenMap`, that is
    /// the slot storage, and none of the slots are free. The field says why,
    /// which for a [`SingleVec`](crate::SingleVec) is a
    /// [`ReserveError`](crate::ReserveError).
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

/// This error says why [`GenMap::try_insert`](crate::GenMap::try_insert) or
/// [`DenseGenMap::try_insert`](crate::DenseGenMap::try_insert) could not
/// insert. Each variant hands the value back so that the caller can keep it.
/// `S` is the map's [`StorageError`](crate::StorageError), or the
/// [`DenseStorageError`](crate::DenseStorageError) of a `DenseGenMap`.
///
/// When the keys have no index left for a new slot and a storage is also
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

/// This error says why [`SecondaryMap::insert`](crate::SecondaryMap::insert),
/// [`DenseSecondaryMap::insert`](crate::DenseSecondaryMap::insert) or
/// [`SparseSecondaryMap::insert`](crate::SparseSecondaryMap::insert) could
/// not insert. Each variant hands the value back so that the caller can keep
/// it. `S` is the map's
/// [`SecondaryStorageError`](crate::SecondaryStorageError), the
/// [`DenseSecondaryStorageError`](crate::DenseSecondaryStorageError) of a
/// `DenseSecondaryMap`, or `TryReserveError` for a `SparseSecondaryMap`.
pub enum SecondaryInsertError<T, S> {
    /// The value stored at the key's index was inserted under a different
    /// generation, and the config's
    /// [`ReplaceStrategy`](crate::ReplaceStrategy) kept that value.
    Refused(T),

    /// The key's index is the largest value of the index type. No map gives
    /// a slot that index, so only a hand-built key can have it.
    IndexReserved(T),

    /// A storage could not make room for the value. For a `SecondaryMap`, it
    /// is the storage of the slots up to the key's index, and for a
    /// `SparseSecondaryMap`, it is the `HashMap` of its values. The second
    /// field says why.
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

/// This error says why
/// [`GenMap::try_insert_with_key`](crate::GenMap::try_insert_with_key) or
/// [`DenseGenMap::try_insert_with_key`](crate::DenseGenMap::try_insert_with_key)
/// could not insert. The map can be full before the closure runs, or the
/// closure can refuse to make a value, and the variant says which of the two
/// happened. `E` is the closure's error, and `S` is the map's
/// [`StorageError`](crate::StorageError), or the
/// [`DenseStorageError`](crate::DenseStorageError) of a `DenseGenMap`.
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
    /// use gen_map::{FullError, GenMap, ReserveError};
    ///
    /// #[derive(Debug)]
    /// enum MyError {
    ///     Full(FullError<ReserveError>),
    ///     Parse,
    /// }
    ///
    /// impl From<FullError<ReserveError>> for MyError {
    ///     fn from(e: FullError<ReserveError>) -> Self {
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

/// This error says why the `get_disjoint_mut` method of a
/// [`GenMap`](crate::GenMap), [`SecondaryMap`](crate::SecondaryMap),
/// [`DenseGenMap`](crate::DenseGenMap),
/// [`DenseSecondaryMap`](crate::DenseSecondaryMap) or
/// [`SparseSecondaryMap`](crate::SparseSecondaryMap) could not hand out its
/// references.
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

/// Checks the keys given to a map's `get_disjoint_mut`, where `has_value`
/// says whether the map has a value for a key. Keys point at the same slot
/// exactly when their indices are equal.
///
/// # Errors
///
/// Returns [`GetDisjointMutError::InvalidKey`] if the map has no value for one
/// of the keys, and [`GetDisjointMutError::OverlappingKeys`] if two of the keys
/// have the same index.
pub(crate) fn check_disjoint_keys<K: KeyConfig>(
    keys: &[Key<K>],
    mut has_value: impl FnMut(Key<K>) -> bool,
) -> Result<(), GetDisjointMutError> {
    for (i, key) in keys.iter().enumerate() {
        if !has_value(*key) {
            return Err(GetDisjointMutError::InvalidKey);
        }
        if keys[..i].iter().any(|earlier| earlier.idx() == key.idx()) {
            return Err(GetDisjointMutError::OverlappingKeys);
        }
    }
    Ok(())
}

/// This error says why the `get_disjoint_mut_at` method of a
/// [`GenMap`](crate::GenMap), [`SecondaryMap`](crate::SecondaryMap),
/// [`DenseGenMap`](crate::DenseGenMap),
/// [`DenseSecondaryMap`](crate::DenseSecondaryMap) or
/// [`SparseSecondaryMap`](crate::SparseSecondaryMap) could not hand out its
/// references.
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

/// Checks the indices given to a map's `get_disjoint_mut_at`, where
/// `has_value` says whether the slot at an index holds a value.
///
/// # Errors
///
/// Returns [`GetDisjointMutAtError::NoValue`] if the slot at one of the
/// indices holds no value, and [`GetDisjointMutAtError::OverlappingIndices`]
/// if two of the indices are the same.
pub(crate) fn check_disjoint_idxs<I: PartialEq>(
    idxs: &[I],
    mut has_value: impl FnMut(&I) -> bool,
) -> Result<(), GetDisjointMutAtError> {
    for (i, idx) in idxs.iter().enumerate() {
        if !has_value(idx) {
            return Err(GetDisjointMutAtError::NoValue);
        }
        if idxs[..i].contains(idx) {
            return Err(GetDisjointMutAtError::OverlappingIndices);
        }
    }
    Ok(())
}

/// This error says which storage of a dense map could not make room, and it
/// holds that storage's error. The first type parameter is the error of the
/// slot storage, and the second is the error of the
/// [`PairStorage`](crate::PairStorage) that holds the keys and values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenseError<S, P> {
    /// The slot storage could not make room for another slot.
    Slots(S),
    /// The pair storage could not make room for another key and value.
    Pairs(P),
}

impl<S: fmt::Display, P: fmt::Display> fmt::Display for DenseError<S, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Slots(error) => write!(f, "the slot storage is full: {error}"),
            Self::Pairs(error) => write!(f, "the pair storage is full: {error}"),
        }
    }
}

/// This error says why a [`SingleVec`](crate::SingleVec) or a
/// [`PairVec`](crate::PairVec) could not make room for more items. It needs
/// the `alloc` feature.
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReserveError {
    /// The vec would need room for more items than its length type can count,
    /// or a buffer that takes more than `isize::MAX` bytes.
    CapacityOverflow,
    /// The allocator could not allocate a buffer with this layout.
    AllocError(Layout),
}

#[cfg(feature = "alloc")]
impl ReserveError {
    /// Panics for a capacity overflow, and calls `handle_alloc_error` for a
    /// failed allocation, which aborts the program by default.
    #[cold]
    #[inline(never)]
    pub(crate) fn handle(self) -> ! {
        match self {
            Self::CapacityOverflow => panic!("capacity overflow"),
            Self::AllocError(layout) => handle_alloc_error(layout),
        }
    }
}

#[cfg(feature = "alloc")]
impl fmt::Display for ReserveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityOverflow => f.write_str("the vec cannot hold that many items"),
            Self::AllocError(layout) => write!(
                f,
                "the allocator could not allocate {} bytes for a buffer",
                layout.size()
            ),
        }
    }
}
