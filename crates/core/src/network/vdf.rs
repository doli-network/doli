//! VDF (Verifiable Delay Function) parameters for DOLI networks
//!
//! Cached VDF discriminants, iteration counts, and network-specific seeds.

use std::sync::OnceLock;

use vdf::VdfParams;

use super::Network;

/// Cached VDF parameters for each network (generated once, reused forever)
static MAINNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();
static TESTNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();
static DEVNET_VDF_PARAMS: OnceLock<VdfParams> = OnceLock::new();

impl Network {
    /// Check if VDF is enabled for this network
    ///
    /// All networks use VDF (hash-chain based) for Proof of Time.
    /// Devnet uses faster parameters for testing.
    pub fn vdf_enabled(&self) -> bool {
        match self {
            Network::Mainnet => true,
            Network::Testnet => true,
            Network::Devnet => true, // Enabled with fast hash-chain VDF (~700ms)
        }
    }

    /// Get VDF iterations for block production
    ///
    /// These values are calibrated for practical block production times
    /// based on the network-specific discriminant size.
    ///
    /// Note: Check vdf_enabled() first - if false, VDF is skipped entirely.
    ///
    /// Configurable via `DOLI_VDF_ITERATIONS` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn vdf_iterations(&self) -> u64 {
        self.params().vdf_iterations
    }

    /// Get VDF discriminant size in bits for this network
    ///
    /// The discriminant size determines security vs. speed tradeoff:
    /// - Larger discriminants are more secure but slower
    /// - Smaller discriminants are faster but provide less security
    ///
    /// Mainnet and Testnet use production-grade security.
    /// Devnet uses minimal security for fast development.
    pub fn vdf_discriminant_bits(&self) -> usize {
        match self {
            Network::Mainnet => 2048, // Production security (~112-bit)
            Network::Testnet => 2048, // Same as mainnet (production security)
            Network::Devnet => 256,   // Minimal security, very fast
        }
    }

    /// Get VDF seed for deterministic discriminant generation
    ///
    /// Each network uses a unique seed to generate its discriminant,
    /// ensuring proofs from different networks are incompatible.
    pub fn vdf_seed(&self) -> &'static [u8] {
        match self {
            Network::Mainnet => b"DOLI_VDF_DISCRIMINANT_V1_MAINNET",
            Network::Testnet => b"DOLI_VDF_DISCRIMINANT_V1_TESTNET",
            Network::Devnet => b"DOLI_VDF_DISCRIMINANT_V1_DEVNET",
        }
    }

    /// Get cached VDF parameters for this network
    ///
    /// The VDF discriminant is expensive to generate (involves finding large primes),
    /// so we cache it once per network. All subsequent calls return the cached params.
    ///
    /// This is the recommended way to get VDF parameters for computation and verification.
    pub fn vdf_params(&self) -> &'static VdfParams {
        match self {
            Network::Mainnet => MAINNET_VDF_PARAMS.get_or_init(|| {
                VdfParams::with_seed(self.vdf_discriminant_bits(), self.vdf_seed())
            }),
            Network::Testnet => TESTNET_VDF_PARAMS.get_or_init(|| {
                VdfParams::with_seed(self.vdf_discriminant_bits(), self.vdf_seed())
            }),
            Network::Devnet => DEVNET_VDF_PARAMS.get_or_init(|| {
                VdfParams::with_seed(self.vdf_discriminant_bits(), self.vdf_seed())
            }),
        }
    }

    /// Get VDF target time for this network (in milliseconds)
    ///
    /// With Epoch Lookahead selection (deterministic round-robin), VDF only
    /// needs to prove presence (heartbeat), not prevent grinding.
    ///
    /// Grinding prevention comes from:
    /// 1. Leaders determined at epoch start (not per-slot)
    /// 2. Selection uses slot number + bond distribution, NOT prev_hash
    /// 3. Attackers cannot influence future leader selection
    ///
    /// | Network | Slot  | VDF Target | Purpose                    |
    /// |---------|-------|------------|----------------------------|
    /// | Mainnet | 10s   | ~55ms      | Block VDF proof            |
    /// | Testnet | 10s   | ~55ms      | Block VDF proof            |
    /// | Devnet  | 1s    | ~55ms      | Fast development cycles    |
    pub fn vdf_target_time_ms(&self) -> u64 {
        match self {
            Network::Mainnet => 55, // ~55ms (800K iterations)
            Network::Testnet => 55, // ~55ms (800K iterations)
            Network::Devnet => 55,  // ~55ms (800K iterations)
        }
    }

    /// Get heartbeat VDF iterations for this network
    ///
    /// Hash-chain VDF iterations calibrated for target time:
    /// - 800K iterations ≈ 55ms on modern hardware
    ///
    /// Configurable via `DOLI_HEARTBEAT_VDF_ITERATIONS` environment variable (devnet only).
    /// Locked for mainnet to ensure consensus compatibility.
    pub fn heartbeat_vdf_iterations(&self) -> u64 {
        self.params().heartbeat_vdf_iterations
    }

    /// VDF iterations for registration proof
    ///
    /// Configurable via `DOLI_VDF_REGISTER_ITERATIONS` environment variable (devnet only).
    /// Locked for mainnet to ensure anti-Sybil protection.
    pub fn vdf_register_iterations(&self) -> u64 {
        self.params().vdf_register_iterations
    }
}
