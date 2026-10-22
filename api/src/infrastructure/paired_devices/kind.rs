//! `DeviceKind` (design `kiosk-shared-phone.md` §7.5): what differs between the paired devices'
//! secrets and limits. Everything else about pairing is the one core (`pairing.rs`).
//!
//! The drawer agent's values are today's (`agent_service.rs:93`, `:115`, `:131`), so the drawer's
//! tokens and its `auth_attempts` keys stay valid when it moves onto the core (BT-2).

/// The kinds of paired device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// The counter drawer agent (`device_agents`).
    DrawerAgent,
    /// The branch's own customer tablet (`branch_tablets`).
    BranchTablet,
}

/// How many active devices of a kind one branch may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerBranch {
    /// At most one that is not revoked (`device_agents_one_active_per_branch_idx`).
    One,
    /// Any number (T-D4).
    Many,
}

impl DeviceKind {
    /// Every token of this kind starts with it: `xcda_` or `xcbt_`. A token with the other
    /// kind's prefix is refused before any lookup.
    pub const fn token_prefix(self) -> &'static str {
        match self {
            Self::DrawerAgent => "xcda_",
            Self::BranchTablet => "xcbt_",
        }
    }

    /// The text whose `DeviceSecret::hash` is the `auth_attempts.key` counting pair attempts
    /// from `address`: `PAIR:<address>` or `TABLET_PAIR:<address>`.
    pub fn pair_key(self, address: &str) -> String {
        match self {
            Self::DrawerAgent => format!("PAIR:{address}"),
            Self::BranchTablet => format!("TABLET_PAIR:{address}"),
        }
    }

    /// The text whose `DeviceSecret::hash` is the `auth_attempts.key` counting failed token
    /// checks from `address`: `AGENT:<address>` or `TABLET:<address>`.
    pub fn token_key(self, address: &str) -> String {
        match self {
            Self::DrawerAgent => format!("AGENT:{address}"),
            Self::BranchTablet => format!("TABLET:{address}"),
        }
    }

    /// One drawer agent per branch; any number of tablets (T-D4).
    pub const fn per_branch(self) -> PerBranch {
        match self {
            Self::DrawerAgent => PerBranch::One,
            Self::BranchTablet => PerBranch::Many,
        }
    }
}
