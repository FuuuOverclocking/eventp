// Code from `likely_stable@0.1.3`, since `std::intrinsics::unlikely`
// is not stable at this moment.
pub(crate) const fn unlikely(b: bool) -> bool {
    #[allow(clippy::needless_bool)]
    if (1i32).checked_div(if b { 0 } else { 1 }).is_none() {
        true
    } else {
        false
    }
}
