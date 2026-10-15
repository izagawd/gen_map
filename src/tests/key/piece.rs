use crate::{u1, KeyPiece, NonMax, NonMaxU1, NonZeroU1};
use core::mem::{align_of, size_of};
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
fn wrapping_sub_subtracts_and_wraps_like_the_integers() {
    assert_eq!(KeyPiece::wrapping_sub(3u8, 2), 1);
    assert_eq!(KeyPiece::wrapping_sub(0u8, 1), u8::MAX);
    assert_eq!(KeyPiece::wrapping_sub(1u16, 2), u16::MAX);
    assert_eq!(KeyPiece::wrapping_sub(u32::MAX, 1), u32::MAX - 1);
    assert_eq!(KeyPiece::wrapping_sub(0u64, 1), u64::MAX);
    assert_eq!(KeyPiece::wrapping_sub(0u128, 1), u128::MAX);
    assert_eq!(KeyPiece::wrapping_sub(0usize, 1), usize::MAX);
}

#[test]
fn the_largest_value_is_odd_so_an_even_value_plus_one_fits() {
    fn check<P: KeyPiece>(even: P) {
        assert!(!even.is_odd());
        assert_eq!(even.checked_add(P::ONE), Some(even.wrapping_add(P::ONE)));
    }
    check(u1::MIN);
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
    check(u1::MIN);
    check(u1::MAX);
    check(0u8);
    check(u8::MAX);
    check(u16::MAX);
    check(u32::MAX);
    check(usize::MAX);
}

#[test]
fn the_non_zero_and_non_max_forms_round_trip() {
    fn check<P: KeyPiece>(v: P) {
        match v.into_non_zero() {
            Some(non_zero) => {
                assert_eq!(P::from_non_zero(non_zero), v);
                assert_eq!(unsafe { v.into_non_zero_unchecked() }, non_zero);
            }
            None => assert_eq!(v, P::ZERO),
        }
        match v.into_non_max() {
            Some(non_max) => {
                assert_eq!(P::from_non_max(non_max), v);
                assert_eq!(unsafe { v.into_non_max_unchecked() }, non_max);
            }
            None => assert_eq!(v, P::MAX),
        }
    }
    check(u1::MIN);
    check(u1::MAX);
    for v in [0u8, 1, 200, u8::MAX] {
        check(v);
    }
    check(0u32);
    check(u32::MAX);
    check(u128::MAX - 1);
    check(u128::MAX);
    check(usize::MAX);
}

#[test]
fn non_max_zero_is_zero_in_its_non_max_form() {
    fn check<P: KeyPiece>() {
        assert_eq!(P::ZERO.into_non_max(), Some(P::NON_MAX_ZERO));
    }
    check::<u1>();
    check::<u8>();
    check::<u16>();
    check::<u32>();
    check::<u64>();
    check::<u128>();
    check::<usize>();
}

#[test]
fn non_max_holds_every_value_but_the_largest() {
    assert_eq!(NonMax::<u8>::new(0).map(NonMax::get), Some(0));
    assert_eq!(NonMax::<u8>::new(254).map(NonMax::get), Some(254));
    assert_eq!(NonMax::<u8>::new(u8::MAX), None);
    assert_eq!(NonMax::<u128>::new(u128::MAX), None);
    assert_eq!(unsafe { NonMax::<usize>::new_unchecked(7) }.get(), 7);
    assert!(NonMax::<u32>::new(1).unwrap() < NonMax::<u32>::new(2).unwrap());
    assert_eq!(size_of::<NonMax<u64>>(), 8);
    let (five, six) = (
        NonMax::<u16>::new(5).unwrap(),
        NonMax::<u16>::new(6).unwrap(),
    );
    assert_eq!(std::format!("{five:?} {six}"), "5 6");
}

#[test]
fn u1_has_the_constants_of_a_one_bit_integer() {
    assert_eq!(
        (u1::MIN, u1::MAX, u1::BITS),
        (u1::from(false), u1::from(true), 1)
    );
    assert_eq!(
        (
            <u1 as KeyPiece>::ZERO,
            <u1 as KeyPiece>::ONE,
            <u1 as KeyPiece>::MAX,
            <u1 as KeyPiece>::BITS,
        ),
        (u1::MIN, u1::MAX, u1::MAX, 1)
    );
    assert!(!u1::MIN.is_odd());
    assert!(u1::MAX.is_odd());
    assert!(u1::MIN < u1::MAX);
    assert_eq!(u1::default(), u1::MIN);
}

#[test]
fn u1_adds_and_subtracts_like_a_one_bit_integer() {
    let (zero, one) = (u1::MIN, u1::MAX);
    // Each row holds two `u1`s, their wrapping sum and their checked sum.
    for (lhs, rhs, sum, checked) in [
        (zero, zero, zero, Some(zero)),
        (zero, one, one, Some(one)),
        (one, zero, one, Some(one)),
        (one, one, zero, None),
    ] {
        assert_eq!(lhs.wrapping_add(rhs), sum);
        assert_eq!(lhs.checked_add(rhs), checked);
        // Subtracting `rhs` from the sum undoes the add, even when it wrapped.
        assert_eq!(sum.wrapping_sub(rhs), lhs);
    }
}

#[test]
fn u1_converts_like_a_one_bit_integer() {
    assert_eq!(u1::MIN.into_usize(), Some(0));
    assert_eq!(u1::MAX.into_usize(), Some(1));
    assert_eq!(unsafe { u1::MAX.into_usize_unchecked() }, 1);
    assert_eq!(u1::from_usize(0), Some(u1::MIN));
    assert_eq!(u1::from_usize(1), Some(u1::MAX));
    assert_eq!(u1::from_usize(2), None);
    assert_eq!(u1::from_usize(usize::MAX), None);
    assert_eq!(unsafe { u1::from_usize_unchecked(0) }, u1::MIN);
    assert_eq!(unsafe { u1::from_usize_unchecked(1) }, u1::MAX);

    assert_eq!(u1::MIN.into_u128(), 0);
    assert_eq!(u1::MAX.into_u128(), 1);
    assert_eq!(u1::from_u128(0), Some(u1::MIN));
    assert_eq!(u1::from_u128(1), Some(u1::MAX));
    assert_eq!(u1::from_u128(2), None);
    assert_eq!(u1::from_u128(u128::MAX), None);
    assert_eq!(unsafe { u1::from_u128_unchecked(0) }, u1::MIN);
    assert_eq!(unsafe { u1::from_u128_unchecked(1) }, u1::MAX);

    assert!(bool::from(u1::MAX));
    assert!(!bool::from(u1::MIN));
}

#[test]
fn u1_takes_the_space_of_a_bool_and_prints_as_a_number() {
    assert_eq!(size_of::<u1>(), 1);
    assert_eq!(std::format!("{} {}", u1::MIN, u1::MAX), "0 1");
    assert_eq!(std::format!("{:?} {:?}", u1::MIN, u1::MAX), "0 1");
}

#[test]
fn the_non_zero_and_non_max_forms_of_u1_take_no_space() {
    assert_eq!(u1::MAX.into_non_zero(), Some(NonZeroU1));
    assert_eq!(u1::MIN.into_non_zero(), None);
    assert_eq!(NonZeroU1.get(), u1::MAX);
    assert_eq!(u1::MIN.into_non_max(), Some(NonMaxU1));
    assert_eq!(u1::MAX.into_non_max(), None);
    assert_eq!(NonMaxU1.get(), u1::MIN);

    assert_eq!((size_of::<NonZeroU1>(), align_of::<NonZeroU1>()), (0, 1));
    assert_eq!((size_of::<NonMaxU1>(), align_of::<NonMaxU1>()), (0, 1));
    assert_eq!(
        std::format!("{NonZeroU1:?} {NonMaxU1:?} {NonZeroU1} {NonMaxU1}"),
        "1 0 1 0"
    );
}
