# Changelog

## 0.3.0 (2026-09-28)

### Added

- `SecondaryMap` has been added, and it associates values with the keys of a `GenMap`. It is configurable through the `SecondaryMapConfig` trait.

### Changed

- `Config` has been split into `MapConfig` and `GenMapConfig<S>`.
- A config can now restrict which value types a map holds, through bounds on `GenSlotItem::Value`. 
- The config of a `GenMap<T, C>` is now bounded by `MapConfigFor<T>`.
- `DefaultConfig` has been replaced by `DefaultMapConfig`.
- `KeyLayout` has been replaced by `KeyConfig`, and `SplitRepr` and `PackedRepr` have been removed.
- `Split` is now `Split<Idx, Gen>`, with both parameters defaulting to `u32`.
- `Packed` now picks its own index and generation types, and it no longer accepts `usize` as `R`.
- `Key<K>` now takes a key config instead of a map config. Maps whose configs have the same key config share a key type, so one map accepts another's keys.
- `Key`'s parameter now defaults to `DefaultKeyConfig` even without the `alloc` feature.
- `Key::generation` now returns an `Odd`. `Key::from_raw_parts` has been replaced by `Key::from_repr` and `KeyConfig::pack`, and `Key::non_zero_generation` has been removed.
- `GenMap::reattach` now returns `Result<(), T>`, and `GenMap::detach` now accepts a key whose generation is the largest one its key config can hold.
- A map no longer uses the largest value of its index type as an index.
- `GenMap::new` and `GenMap::new_with_config` are no longer `const`.
- `Slot<T, C>` has become `Slot<G, T, U>`, and it can now be used directly, outside a map. It holds a generation of type `G` together with a `T` while the generation is odd, or a `U` while it is even.
- `SlotStorage` and `ReserveStorage` now have no type parameter. `SlotStorage::EMPTY` has been replaced by `SlotStorage::empty`, and `SlotStorage::ensure_room` now takes the number of items to make room for.
- `SlotStorage` no longer requires `IntoIterator`, and `GenMap` implements `IntoIterator` only when its storage does.
- `KeyPiece` is now sealed.
- `GenMap` and its slots now use less memory.
- The docs on docs.rs now show which items need which feature.
