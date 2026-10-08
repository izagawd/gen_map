use crate::{Even, Odd};
use core::mem::size_of;

#[test]
fn odd_accepts_only_odd_numbers() {
    assert_eq!(Odd::new(1u8).map(|odd| odd.get().get()), Some(1));
    assert_eq!(Odd::new(u8::MAX).map(|odd| odd.get().get()), Some(u8::MAX));
    assert!(Odd::new(0u8).is_none());
    assert!(Odd::new(2u32).is_none());
}

#[test]
fn even_accepts_only_even_numbers() {
    assert_eq!(Even::new(0u8).map(Even::get), Some(0));
    assert_eq!(Even::new(254u8).map(Even::get), Some(254));
    assert!(Even::new(1u8).is_none());
    assert!(Even::new(u32::MAX).is_none());
}

#[test]
fn new_unchecked_keeps_the_number() {
    // SAFETY: 5 is odd and 6 is even.
    let (odd, even) = unsafe { (Odd::new_unchecked(5u8), Even::new_unchecked(6u8)) };
    assert_eq!(odd.get().get(), 5);
    assert_eq!(even.get(), 6);
    assert_eq!(Some(odd), Odd::new(5));
    assert_eq!(Some(even), Even::new(6));
}

#[test]
fn even_zero_is_zero() {
    assert_eq!(Even::<u16>::ZERO.get(), 0);
    assert_eq!(Even::new(0u16), Some(Even::ZERO));
}

#[test]
fn the_number_after_an_even_one_is_odd() {
    assert_eq!(Even::<u8>::ZERO.next().get().get(), 1);
    assert_eq!(Even::new(254u8).unwrap().next().get().get(), u8::MAX);
}

#[test]
fn the_number_after_an_odd_one_is_even_and_the_largest_wraps_to_zero() {
    assert_eq!(Odd::new(1u8).unwrap().wrapping_next().get(), 2);
    assert_eq!(Odd::new(253u8).unwrap().wrapping_next().get(), 254);
    assert_eq!(Odd::new(u8::MAX).unwrap().wrapping_next(), Even::ZERO);
    assert_eq!(Odd::new(u128::MAX).unwrap().wrapping_next(), Even::ZERO);
}

#[test]
fn the_number_before_an_odd_one_is_even() {
    assert_eq!(Odd::new(1u8).unwrap().previous(), Even::ZERO);
    assert_eq!(Odd::new(u8::MAX).unwrap().previous().get(), 254);
}

#[test]
fn the_number_before_an_even_one_is_odd_and_zero_wraps_to_the_largest() {
    assert_eq!(Even::new(2u8).unwrap().wrapping_previous().get().get(), 1);
    assert_eq!(Even::<u8>::ZERO.wrapping_previous().get().get(), u8::MAX);
    assert_eq!(
        Even::<u128>::ZERO.wrapping_previous().get().get(),
        u128::MAX
    );
}

#[test]
fn previous_undoes_next() {
    let even = Even::new(40u16).unwrap();
    assert_eq!(even.next().previous(), even);
    let odd = Odd::new(41u16).unwrap();
    assert_eq!(odd.wrapping_next().wrapping_previous(), odd);
    assert_eq!(
        Odd::new(u16::MAX)
            .unwrap()
            .wrapping_next()
            .wrapping_previous(),
        Odd::new(u16::MAX).unwrap()
    );
}

#[test]
fn an_optional_odd_is_as_large_as_its_number() {
    assert_eq!(size_of::<Option<Odd<u8>>>(), 1);
    assert_eq!(size_of::<Option<Odd<u32>>>(), 4);
    assert_eq!(size_of::<Option<Odd<usize>>>(), size_of::<usize>());
}

#[test]
fn wrappers_are_ordered_by_their_numbers() {
    let odds = [1u32, 3, 99].map(|n| Odd::new(n).unwrap());
    assert!(odds.windows(2).all(|pair| pair[0] < pair[1]));
    let evens = [0u32, 2, 100].map(|n| Even::new(n).unwrap());
    assert!(evens.windows(2).all(|pair| pair[0] < pair[1]));
}
