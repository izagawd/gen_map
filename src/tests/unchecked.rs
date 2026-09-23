use crate::GenMap;
use std::string::{String, ToString};
use std::vec::Vec;

#[test]
fn get_unchecked_returns_the_right_value() {
    let mut map = GenMap::new();
    let k1 = map.insert(10);
    let k2 = map.insert(20);
    let k3 = map.insert(30);

    unsafe {
        assert_eq!(*map.get_unchecked(k1), 10);
        assert_eq!(*map.get_unchecked(k2), 20);
        assert_eq!(*map.get_unchecked(k3), 30);
    }
}

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
fn get_unchecked_after_remove_and_reinsert() {
    let mut map = GenMap::new();
    let k1 = map.insert(100);
    let k2 = map.insert(200);

    map.remove(k1);
    let k3 = map.insert(300);
    assert_eq!(k3.idx, k1.idx);

    unsafe {
        assert_eq!(*map.get_unchecked(k2), 200);
        assert_eq!(*map.get_unchecked(k3), 300);
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

#[test]
fn get_unchecked_mut_on_several_keys() {
    let mut map = GenMap::new();
    let k1 = map.insert("hello".to_string());
    let k2 = map.insert("world".to_string());

    unsafe {
        map.get_unchecked_mut(k1).push('!');
        map.get_unchecked_mut(k2).push('?');
    }

    assert_eq!(map[k1], "hello!");
    assert_eq!(map[k2], "world?");
}
