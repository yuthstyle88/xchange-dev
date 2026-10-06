//! Which way the foreign currency moves, seen from the shop.

/// Which way the foreign currency moves, seen from the shop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The shop buys foreign currency and pays baht out.
    Buy,
    /// The shop sells foreign currency and takes baht in.
    Sell,
}
