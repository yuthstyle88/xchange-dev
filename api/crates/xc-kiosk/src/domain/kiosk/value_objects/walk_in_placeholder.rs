//! The desk's passport skip (design `kiosk-shared-phone.md` D20): the one place that knows its values.

/// The placeholder passport the desk sends when staff skip the passport check (`web/src/screens/desk/steps/customer/step.tsx`,
/// #539): passport number `WALKIN`, nationality `XXX`. A save on it is never "a different customer" (D13).
pub struct WalkInPlaceholder;

impl WalkInPlaceholder {
    pub const PASSPORT_NO: &'static str = "WALKIN";
    pub const NATIONALITY: &'static str = "XXX";

    /// Whether `(passport_no, nationality)` is the desk's skip placeholder.
    pub fn is(passport_no: &str, nationality: &str) -> bool {
        passport_no.trim().eq_ignore_ascii_case(Self::PASSPORT_NO)
            && nationality.trim().eq_ignore_ascii_case(Self::NATIONALITY)
    }
}
