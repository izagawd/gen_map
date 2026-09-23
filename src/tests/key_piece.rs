use crate::KeyPiece;
use core::num::NonZero;

#[test]
fn wrapping_add_adds_and_wraps_like_the_integers() {
    assert_eq!(KeyPiece::wrapping_add(1u8, 2), 3);
    assert_eq!(KeyPiece::wrapping_add(u8::MAX, 1), 0);
    assert_eq!(KeyPiece::wrapping_add(u16::MAX, 2), 1);
    assert_eq!(KeyPiece::wrapping_add(u32::MAX - 1, 1), u32::MAX);
    assert_eq!(KeyPiece::wrapping_add(u64::MAX, 1), 0);
    assert_eq!(KeyPiece::wrapping_add(u128::MAX, 1), 0);
    assert_eq!(KeyPiece::wrapping_add(usize::MAX, 1), 0);
}

#[test]
fn the_largest_value_is_odd_so_an_even_value_plus_one_fits() {
    fn check<P: KeyPiece>(even: P) {
        assert!(!even.is_odd());
        assert_eq!(even.checked_add(P::ONE), Some(even.wrapping_add(P::ONE)));
    }
    check(0u8);
    check(254u8);
    check(u16::MAX - 1);
    check(u32::MAX - 1);
    check(u64::MAX - 1);
    check(u128::MAX - 1);
    check(usize::MAX - 1);
}

#[test]
fn into_usize_unchecked_agrees_with_into_usize() {
    for v in [0u8, 1, 200, u8::MAX] {
        assert_eq!(unsafe { v.into_usize_unchecked() }, v.into_usize().unwrap());
    }
    for v in [0u64, 1, 1 << 40, usize::MAX as u64] {
        assert_eq!(unsafe { v.into_usize_unchecked() }, v.into_usize().unwrap());
    }
    let v = usize::MAX as u128;
    assert_eq!(unsafe { v.into_usize_unchecked() }, usize::MAX);
}

#[test]
fn from_usize_unchecked_agrees_with_from_usize() {
    for v in [0usize, 1, 255] {
        assert_eq!(
            unsafe { u8::from_usize_unchecked(v) },
            u8::from_usize(v).unwrap()
        );
    }
    for v in [0usize, 65_535, usize::MAX] {
        assert_eq!(unsafe { u128::from_usize_unchecked(v) }, v as u128);
        assert_eq!(unsafe { usize::from_usize_unchecked(v) }, v);
    }
}

#[test]
fn into_non_zero_unchecked_agrees_with_into_non_zero() {
    assert_eq!(
        unsafe { 1u8.into_non_zero_unchecked() },
        NonZero::new(1).unwrap()
    );
    assert_eq!(
        unsafe { u8::MAX.into_non_zero_unchecked() },
        NonZero::new(u8::MAX).unwrap()
    );
    assert_eq!(
        unsafe { 7u32.into_non_zero_unchecked() },
        7u32.into_non_zero().unwrap()
    );
    assert_eq!(
        unsafe { u128::MAX.into_non_zero_unchecked() },
        NonZero::new(u128::MAX).unwrap()
    );
    assert_eq!(
        u32::from_non_zero(unsafe { 9u32.into_non_zero_unchecked() }),
        9
    );
}

#[test]
fn the_conversions_round_trip() {
    fn check<P: KeyPiece>(v: P) {
        let position = unsafe { v.into_usize_unchecked() };
        assert_eq!(unsafe { P::from_usize_unchecked(position) }, v);
        assert_eq!(P::from_usize(position), Some(v));
    }
    check(0u8);
    check(u8::MAX);
    check(u16::MAX);
    check(u32::MAX);
    check(usize::MAX);
}
