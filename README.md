# gen_map

A customizable generational map that returns a `Key` upon inserting a value. The key can be used to later access or remove the value, and removing a value bumps its slot's generation, so the old key no longer matches.
The operations for inserting, removing and accessing a value are all O(1).

The crate is `no_std`.

```toml
[dependencies]
gen_map = "0.2.2"
```

## Example

```rust
use gen_map::GenMap;

let mut map = GenMap::new();
let a = map.insert("a");
let b = map.insert("b");

assert_eq!(map[a], "a");
assert_eq!(map[b], "b");
assert_eq!(map.remove(a), Some("a"));
assert!(map.get(a).is_none()); // A removed key never matches again.

let c = map.insert("c"); // This takes the slot `a` had, but under a new key.
assert_ne!(a, c);

for (key, value) in &map {
println!("{key:?} = {value}");
}
```

## Configuring the map

A `KeyConfig` picks the key's index and generation types and how the key
stores them. A `MapConfig` picks the key config, where the slots live and
what happens when a slot's generation runs out.

```rust
use gen_map::{GenMap, KeyConfig, MapConfig, Packed, SlotItem};

/// Four byte keys with 24 bits of index and 8 bits of generation.
struct CompactKey;

impl KeyConfig for CompactKey {
    type Idx = u32;
    type Gen = u8;
    type Layout = Packed<u32, 8>;
}

struct CompactMap;

// `S` is the slot the map keeps each value in.
impl<S: SlotItem> MapConfig<S> for CompactMap {
    type KeyConfig = CompactKey;
    type Storage = Vec<S>;
}

let mut map = GenMap::<&str, CompactMap>::new_with_config();
let key = map.insert("a");
assert_eq!(core::mem::size_of_val(&key), 4);
```

The [documentation](https://docs.rs/gen_map) covers the rest, such as key
layouts, storage, what happens when a generation runs out, and limiting which
maps can use a config.

## Cargo features

- `alloc` (on by default): `Vec` storage and the default config. Turn it off
  and use `arrayvec` to run without an allocator.
- `arrayvec`: `ArrayVec` storage, which has a fixed capacity and never
  allocates.
- `smallvec`: `SmallVec` storage, which keeps a few slots inline before it
  allocates. It uses a beta of smallvec 2.0, so it is not covered by semver.

## License

gen_map is released under the MIT license.