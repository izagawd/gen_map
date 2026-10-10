use crate::config::KeyConfig;
use crate::key::parity::Odd;

/// A replace strategy decides whether a value inserted into a
/// [`SecondaryMap`](crate::SecondaryMap), a
/// [`DenseSecondaryMap`](crate::DenseSecondaryMap) or a
/// [`SparseSecondaryMap`](crate::SparseSecondaryMap) under a key replaces the
/// value already stored at the key's index, when that value was inserted under
/// a different generation.
///
/// The `insert` of each of these maps only asks the strategy in that case.
/// An empty slot always takes the value, and a value stored under the key's
/// generation is always replaced.
///
/// `K` is the key config of the map's keys. A strategy can read the key's
/// limits through `K`, for example to find the largest generation with
/// [`max_generation`](KeyConfig::max_generation). A strategy can also be
/// implemented only for the key configs it supports.
///
/// # Examples
///
/// The strategy below replaces a value only when the key's generation is at
/// least 10 larger than the slot's.
///
/// ```
/// use gen_map::{KeyConfig, Odd, ReplaceStrategy};
///
/// struct FarNewerWins;
///
/// impl<K: KeyConfig<Gen = u32>> ReplaceStrategy<K> for FarNewerWins {
///     fn replaces(slot: Odd<u32>, key: Odd<u32>) -> bool {
///         key.get().get() >= slot.get().get().saturating_add(10)
///     }
/// }
/// ```
pub trait ReplaceStrategy<K: KeyConfig> {
    /// Returns `true` if a value inserted under a key with generation `key`
    /// replaces a value that was inserted under generation `slot`. The two
    /// generations are never equal, because the map only asks the strategy when
    /// they differ.
    fn replaces(slot: Odd<K::Gen>, key: Odd<K::Gen>) -> bool;
}

/// Replaces the stored value when the key's generation is larger than the
/// generation the value was stored under.
///
/// A larger generation only means a newer key while the map that hands out
/// the keys never starts a slot's generation over. A `GenMap` or a
/// `DenseGenMap` wraps a slot's generation back to zero when
/// `WRAP_ON_OVERFLOW` is `true` in its config, and its
/// [`reset`](crate::GenMap::reset) starts every generation over. After either
/// one, a newer key can have a smaller generation than the key of the stored
/// value, and the secondary map refuses the newer key's insert.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NewerWins;

impl<K: KeyConfig> ReplaceStrategy<K> for NewerWins {
    #[inline]
    fn replaces(slot: Odd<K::Gen>, key: Odd<K::Gen>) -> bool {
        key > slot
    }
}

/// Never replaces a value that was inserted under a different generation.
/// An insert into such a slot is refused until the value is removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ExistingWins;

impl<K: KeyConfig> ReplaceStrategy<K> for ExistingWins {
    #[inline]
    fn replaces(_slot: Odd<K::Gen>, _key: Odd<K::Gen>) -> bool {
        false
    }
}
