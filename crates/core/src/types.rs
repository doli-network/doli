//! Core type definitions

use serde::{Deserialize, Serialize};
use std::fmt;

/// Amount in base units (1 coin = 10^8 base units)
pub type Amount = u64;

/// Block height (0-indexed)
pub type BlockHeight = u64;

/// Slot number (time-based)
pub type Slot = u32;

/// Epoch number (60 slots = 1 epoch)
pub type Epoch = u32;

/// Era number (2,102,400 blocks = 1 era)
pub type Era = u32;

/// Number of decimal places
pub const DECIMALS: u32 = 8;

/// Base units per coin
pub const UNITS_PER_COIN: Amount = 100_000_000;

/// Convert coins to base units
pub const fn coins_to_units(coins: u64) -> Amount {
    coins * UNITS_PER_COIN
}

/// Convert base units to coins (truncates)
pub const fn units_to_coins(units: Amount) -> u64 {
    units / UNITS_PER_COIN
}

/// A wrapper for displaying amounts in human-readable format
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayAmount(pub Amount);

impl fmt::Display for DisplayAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let coins = self.0 / UNITS_PER_COIN;
        let frac = self.0 % UNITS_PER_COIN;
        write!(f, "{}.{:08}", coins, frac)
    }
}

impl fmt::Debug for DisplayAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Amount({})", self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coin_conversion() {
        assert_eq!(coins_to_units(1), 100_000_000);
        assert_eq!(coins_to_units(5), 500_000_000);
        assert_eq!(units_to_coins(100_000_000), 1);
        assert_eq!(units_to_coins(150_000_000), 1); // Truncates
    }

    #[test]
    fn test_display_amount() {
        assert_eq!(DisplayAmount(100_000_000).to_string(), "1.00000000");
        assert_eq!(DisplayAmount(500_000_000).to_string(), "5.00000000");
        assert_eq!(DisplayAmount(123_456_789).to_string(), "1.23456789");
        assert_eq!(DisplayAmount(1).to_string(), "0.00000001");
    }

    #[test]
    fn test_edge_cases() {
        // Zero
        assert_eq!(coins_to_units(0), 0);
        assert_eq!(units_to_coins(0), 0);
        assert_eq!(DisplayAmount(0).to_string(), "0.00000000");

        // Maximum safe values
        let max_coins = u64::MAX / UNITS_PER_COIN;
        assert_eq!(units_to_coins(coins_to_units(max_coins)), max_coins);
    }

    // Property-based tests
    use proptest::prelude::*;

    proptest! {
        /// Conversion roundtrip: coins -> units -> coins preserves value
        #[test]
        fn prop_conversion_roundtrip(coins in 0u64..=(u64::MAX / UNITS_PER_COIN)) {
            let units = coins_to_units(coins);
            let back = units_to_coins(units);
            prop_assert_eq!(coins, back);
        }

        /// units_to_coins truncates correctly (loses fractional coins)
        #[test]
        fn prop_units_to_coins_truncates(coins in 0u64..=(u64::MAX / UNITS_PER_COIN), frac in 0u64..(UNITS_PER_COIN)) {
            let units = coins * UNITS_PER_COIN + frac;
            let back = units_to_coins(units);
            prop_assert_eq!(coins, back);
        }

        /// DisplayAmount always produces valid format (N.NNNNNNNN)
        #[test]
        fn prop_display_amount_format(amount: u64) {
            let display = DisplayAmount(amount).to_string();
            // Must contain exactly one dot
            prop_assert_eq!(display.matches('.').count(), 1);
            // Fractional part must be exactly 8 digits
            let parts: Vec<&str> = display.split('.').collect();
            prop_assert_eq!(parts.len(), 2);
            prop_assert_eq!(parts[1].len(), 8);
            // All characters must be digits or dot
            prop_assert!(display.chars().all(|c| c.is_ascii_digit() || c == '.'));
        }

        /// coins_to_units is monotonic
        #[test]
        fn prop_coins_to_units_monotonic(a in 0u64..=(u64::MAX / UNITS_PER_COIN / 2), b in 0u64..=(u64::MAX / UNITS_PER_COIN / 2)) {
            if a <= b {
                prop_assert!(coins_to_units(a) <= coins_to_units(b));
            } else {
                prop_assert!(coins_to_units(a) > coins_to_units(b));
            }
        }

        /// units_to_coins is monotonic
        #[test]
        fn prop_units_to_coins_monotonic(a: u64, b: u64) {
            if a <= b {
                prop_assert!(units_to_coins(a) <= units_to_coins(b));
            } else {
                prop_assert!(units_to_coins(a) > units_to_coins(b));
            }
        }
    }
}
