use crate::GenMap;
use std::string::String;
use std::vec::Vec;

#[test]
fn get_unchecked_agrees_with_get() {
    let mut map = GenMap::<String>::new();
    let keys: Vec<_> = (0..100)
        .map(|i| map.insert(std::format!("item_{i}")))
        .collect();

    for (i, &k) in keys.iter().enumerate() {
        let checked = map.get(k).unwrap();
        let unchecked = unsafe { map.get_unchecked(k) };
        assert!(core::ptr::eq(checked, unchecked));
        assert_eq!(unchecked, &std::format!("item_{i}"));
    }
}

#[test]
fn get_unchecked_mut_writes_through() {
    let mut map = GenMap::new();
    let k = map.insert(42);

    unsafe {
        *map.get_unchecked_mut(k) = 99;
    }
    assert_eq!(map[k], 99);
}
