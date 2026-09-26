use crate::config::KeyConfig;
use crate::key_layout::KeyLayout;
use crate::key_piece::KeyPiece;
use crate::parity::Odd;

/// Decides whether a value inserted into a
/// [`SecondaryMap`](crate::SecondaryMap) replaces the value in the slot at
/// the key's index, when that value was inserted under a different
/// generation.
///
/// [`SecondaryMap::insert`](crate::SecondaryMap::insert) only asks the
/// strategy in that case. An empty slot always takes the value, and a slot
/// whose generation is the key's always has its value replaced.
///
/// `K` is the key config of the map's keys. A strategy can read the key's
/// layout through it, as [`NewerWinsWrapping`] does to find the largest
/// generation, or be implemented only for the key configs it supports.
///
/// A strategy is a type, and [`replaces`](Self::replaces) takes no `self`,
/// so the strategy a map uses is picked at compile time and the call can be
/// inlined.
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
    /// are never equal.
    fn replaces(slot: Odd<K::Gen>, key: Odd<K::Gen>) -> bool;
}

/// Replaces the slot's value when the key's generation is larger than the
/// slot's.
///
/// This is exact as long as the map that hands out the keys never wraps a
/// generation, which is the default, since
/// [`WRAP_ON_OVERFLOW`](crate::GenMapConfig::WRAP_ON_OVERFLOW) is `false`.
/// After a wrap, a newer key can have a smaller generation than the slot,
/// and its insert is refused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NewerWins;

impl<K: KeyConfig> ReplaceStrategy<K> for NewerWins {
    #[inline]
    fn replaces(slot: Odd<K::Gen>, key: Odd<K::Gen>) -> bool {
        key > slot
    }
}

/// Replaces the slot's value when the key's generation is less than half
/// the generation range ahead of the slot's, counting forward from the
/// slot's generation and wrapping from the largest generation back to zero.
/// It is the [`ReplaceStrategy`](crate::SecondaryMapConfig::ReplaceStrategy)
/// of [`DefaultMapConfig`](crate::DefaultMapConfig).
///
/// Counting forward handles a wrap. With generations from 0 to 15, a key
/// with generation 1 is 2 steps ahead of a slot with generation 15, so its
/// value replaces the slot's. It only works while the two are less than
/// half the range apart. A key with generation 13 is 10 steps ahead of a
/// slot with generation 3, so its insert is refused, even though no wrap
/// happened. With a `u32` generation, half the range is over two billion
/// generations.
///
/// The range is every generation from zero to the key layout's
/// [`max_generation`](KeyLayout::max_generation).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NewerWinsWrapping;

impl<K: KeyConfig> ReplaceStrategy<K> for NewerWinsWrapping {
    #[inline]
    fn replaces(slot: Odd<K::Gen>, key: Odd<K::Gen>) -> bool {
        let slot = K::Gen::from_non_zero(slot.get()).into_u128();
        let key = K::Gen::from_non_zero(key.get()).into_u128();
        let max = <K::Layout as KeyLayout<K::Idx, K::Gen>>::max_generation();
        let max = K::Gen::from_non_zero(max.get()).into_u128();
        // `steps` is how far forward `key` is from `slot`. When the range
        // is every `u128`, one more than `max` does not fit, but then
        // wrapping subtraction counts the steps by itself.
        let (steps, half) = match max.checked_add(1) {
            Some(range) if key >= slot => (key - slot, range / 2),
            Some(range) => (range - (slot - key), range / 2),
            None => (key.wrapping_sub(slot), 1 << 127),
        };
        steps < half
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
