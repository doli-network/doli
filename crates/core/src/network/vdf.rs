//! VDF (Verifiable Delay Function) parameters for DOLI networks
//!
//! Network-specific VDF configuration. All consensus-critical VDF uses
//! the BLAKE3 hash-chain implementation in `heartbeat.rs`.

use super::Network;

impl Network {
    /// Check if VDF is enabled for this network
    ///
    /// All networks use VDF (hash-chain based) for Proof of Time.
    /// Devnet uses faster parameters for testing.
    pub fn vdf_enabled(&self) -> bool {
        match self {
            Network::Mainnet => true,
            Network::Testnet => true,
            Network::Devnet => true,
        }
    }

    /// Get VDF iterations for block production
    ///
    /// These values are calibrated for practical block production times.
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
    pub fn vdf_discriminant_bits(&self) -> usize {
        match self {
            Network::Mainnet => 2048,
            Network::Testnet => 2048,
            Network::Devnet => 256,
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

    /// Get VDF target time for this network (in milliseconds)
    ///
    /// | Network | Slot  | VDF Target | Purpose                    |
    /// |---------|-------|------------|----------------------------|
    /// | Mainnet | 10s   | ~55ms      | Block VDF proof            |
    /// | Testnet | 10s   | ~55ms      | Block VDF proof            |
    /// | Devnet  | 1s    | ~55ms      | Fast development cycles    |
    pub fn vdf_target_time_ms(&self) -> u64 {
        match self {
            Network::Mainnet => 55,
            Network::Testnet => 55,
            Network::Devnet => 55,
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
