use crate::{u1, Even, KeyPiece, Odd};
use core::mem::{align_of, size_of};
use core::panic::{RefUnwindSafe, UnwindSafe};

#[test]
fn odd_accepts_only_odd_numbers() {
    assert_eq!(Odd::new(1u8).map(|odd| odd.get().get()), Some(1));
    assert_eq!(Odd::new(u8::MAX).map(|odd| odd.get().get()), Some(u8::MAX));
    assert!(Odd::new(0u8).is_none());
    assert!(Odd::new(2u32).is_none());
}

#[test]
fn even_accepts_only_even_numbers() {
    assert_eq!(Even::new(0u8).map(|even| even.get().get()), Some(0));
    assert_eq!(Even::new(254u8).map(|even| even.get().get()), Some(254));
    assert!(Even::new(1u8).is_none());
    assert!(Even::new(u32::MAX).is_none());
}

#[test]
fn new_unchecked_keeps_the_number() {
    // SAFETY: 5 is odd and 6 is even.
    let (odd, even) = unsafe { (Odd::new_unchecked(5u8), Even::new_unchecked(6u8)) };
    assert_eq!(odd.get().get(), 5);
    assert_eq!(even.get().get(), 6);
    assert_eq!(Some(odd), Odd::new(5));
    assert_eq!(Some(even), Even::new(6));
}

#[test]
fn even_zero_is_zero() {
    assert_eq!(Even::<u16>::ZERO.get().get(), 0);
    assert_eq!(Even::new(0u16), Some(Even::ZERO));
}

#[test]
fn the_number_after_an_even_one_is_odd() {
    assert_eq!(Even::<u8>::ZERO.next().get().get(), 1);
    assert_eq!(Even::new(254u8).unwrap().next().get().get(), u8::MAX);
}

#[test]
fn the_number_after_an_odd_one_is_even_and_the_largest_wraps_to_zero() {
    assert_eq!(Odd::new(1u8).unwrap().wrapping_next().get().get(), 2);
    assert_eq!(Odd::new(253u8).unwrap().wrapping_next().get().get(), 254);
    assert_eq!(Odd::new(u8::MAX).unwrap().wrapping_next(), Even::ZERO);
    assert_eq!(Odd::new(u128::MAX).unwrap().wrapping_next(), Even::ZERO);
}

#[test]
fn the_number_before_an_odd_one_is_even() {
    assert_eq!(Odd::new(1u8).unwrap().previous(), Even::ZERO);
    assert_eq!(Odd::new(u8::MAX).unwrap().previous().get().get(), 254);
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
fn odd_and_even_integers_have_the_layout_of_their_number() {
    fn check<G: KeyPiece>() {
        let layout = (size_of::<G>(), align_of::<G>());
        assert_eq!((size_of::<Odd<G>>(), align_of::<Odd<G>>()), layout);
        assert_eq!((size_of::<Even<G>>(), align_of::<Even<G>>()), layout);
    }
    check::<u8>();
    check::<u16>();
    check::<u32>();
    check::<u64>();
    check::<u128>();
    check::<usize>();
}

#[test]
fn one_is_the_only_odd_u1_and_zero_the_only_even_one() {
    assert_eq!(Odd::new(u1::MAX).map(|odd| odd.get().get()), Some(u1::MAX));
    assert!(Odd::new(u1::MIN).is_none());
    assert_eq!(Even::new(u1::MIN), Some(Even::ZERO));
    assert_eq!(Even::<u1>::ZERO.get().get(), u1::MIN);
    assert!(Even::new(u1::MAX).is_none());
}

#[test]
fn a_u1_steps_between_zero_and_one_in_both_directions() {
    let odd = Odd::new(u1::MAX).unwrap();
    assert_eq!(Even::<u1>::ZERO.next(), odd);
    assert_eq!(Even::<u1>::ZERO.wrapping_previous(), odd);
    assert_eq!(odd.wrapping_next(), Even::ZERO);
    assert_eq!(odd.previous(), Even::ZERO);
}

#[test]
fn an_odd_or_even_u1_takes_no_space() {
    assert_eq!((size_of::<Odd<u1>>(), align_of::<Odd<u1>>()), (0, 1));
    assert_eq!((size_of::<Even<u1>>(), align_of::<Even<u1>>()), (0, 1));
    assert_eq!(size_of::<Option<Odd<u1>>>(), 1);
    assert_eq!(size_of::<Option<Even<u1>>>(), 1);
}

#[test]
fn wrappers_print_their_numbers() {
    let (odd, even) = (Odd::new(3u32).unwrap(), Even::new(4u32).unwrap());
    assert_eq!(std::format!("{odd:?} {even:?}"), "Odd(3) Even(4)");
    let (odd, even) = (Odd::new(u1::MAX).unwrap(), Even::<u1>::ZERO);
    assert_eq!(std::format!("{odd:?} {even:?}"), "Odd(1) Even(0)");
}

#[test]
fn odd_and_even_keep_the_auto_traits_of_their_number_in_generic_code() {
    fn assert_traits<T: Send + Sync + Unpin + UnwindSafe + RefUnwindSafe>() {}
    fn check<G: KeyPiece>() {
        assert_traits::<Odd<G>>();
        assert_traits::<Even<G>>();
    }
    check::<u1>();
    check::<u32>();
}

#[test]
fn wrappers_are_ordered_by_their_numbers() {
    let odds = [1u32, 3, 99].map(|n| Odd::new(n).unwrap());
    assert!(odds.windows(2).all(|pair| pair[0] < pair[1]));
    let evens = [0u32, 2, 100].map(|n| Even::new(n).unwrap());
    assert!(evens.windows(2).all(|pair| pair[0] < pair[1]));
}
