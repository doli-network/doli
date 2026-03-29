use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Expand `~` or `~/...` to the user's home directory.
/// Shell tilde expansion doesn't happen inside Rust — clap default values
/// and CLI arguments with `~` arrive as literal strings/paths.
pub(crate) fn expand_tilde_path(path: &std::path::Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest)
    } else {
        path.to_path_buf()
    }
}

#[derive(Parser)]
#[command(name = "doli-node")]
#[command(about = "DOLI full node", long_about = None)]
#[command(version = env!("DOLI_VERSION_STRING"))]
pub(crate) struct Cli {
    /// Network to connect to (mainnet, testnet, devnet)
    #[arg(short, long, default_value = "mainnet", global = true)]
    pub network: String,

    /// Configuration file path
    #[arg(short, long, default_value = "config.toml")]
    pub config: PathBuf,

    /// Data directory (overrides network default)
    #[arg(short, long)]
    pub data_dir: Option<PathBuf>,

    /// Log level
    #[arg(long, default_value = "info")]
    pub log_level: String,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Run the node
    Run {
        /// Enable block production
        #[arg(long)]
        producer: bool,

        /// Producer key file
        #[arg(long)]
        producer_key: Option<PathBuf>,

        /// Disable auto-updates
        #[arg(long)]
        no_auto_update: bool,

        /// Only notify about updates, don't apply
        #[arg(long)]
        update_notify_only: bool,

        /// Disable automatic rollback on update failures
        #[arg(long)]
        no_auto_rollback: bool,

        /// P2P listen port (overrides network default)
        #[arg(long)]
        p2p_port: Option<u16>,

        /// External address to advertise to peers (e.g., /ip4/198.51.100.1/tcp/30300).
        /// Use when running multiple nodes on the same host to prevent advertising 127.0.0.1.
        #[arg(long)]
        external_address: Option<String>,

        /// RPC listen port (overrides network default)
        #[arg(long)]
        rpc_port: Option<u16>,

        /// RPC bind address (default: 127.0.0.1, use 0.0.0.0 for public access)
        #[arg(long)]
        rpc_bind: Option<String>,

        /// Metrics server port (mainnet default: 9000, testnet: 19000, devnet: 29000)
        /// Note: Override with network-specific port if needed
        #[arg(long, default_value = "9000")]
        metrics_port: u16,

        /// Bootstrap node(s) to connect to (e.g., /ip4/127.0.0.1/tcp/50300).
        /// Can be specified multiple times for mesh redundancy.
        #[arg(long)]
        bootstrap: Vec<String>,

        /// Disable DHT discovery (only connect to explicitly provided bootstrap nodes)
        /// Use this to isolate test networks from external peers
        #[arg(long)]
        no_dht: bool,

        /// Enable relay server mode (for public/bootstrap nodes).
        /// Allows NAT'd peers to relay connections through this node.
        #[arg(long)]
        relay_server: bool,

        /// DANGEROUS: Skip duplicate key detection during producer startup.
        /// Only use if you are CERTAIN no other instance is running with this key.
        /// Using this incorrectly WILL cause slashing (100% bond loss).
        #[arg(long)]
        force_start: bool,

        /// Disable snap sync (use header-first only). For testing/debugging.
        #[arg(long)]
        no_snap_sync: bool,

        /// Skip all interactive confirmations (for automation/scripts).
        /// Implies acceptance of --force-start warning when used together.
        #[arg(long)]
        yes: bool,

        /// Path to chainspec JSON file (overrides built-in network config)
        /// Use scripts/generate_chainspec.sh to create from wallet files
        #[arg(long)]
        chainspec: Option<PathBuf>,

        /// Archive blocks to a directory for disaster recovery.
        /// Each block is stored as a file with atomic writes.
        /// Example: --archive-to /home/ilozada/.doli/mainnet/archive
        #[arg(long)]
        archive_to: Option<PathBuf>,

        /// Start syncing from a trusted checkpoint height (skip earlier blocks).
        /// Use with --checkpoint-hash for fast initial sync from a known-good state.
        #[arg(long)]
        checkpoint_height: Option<u64>,

        /// Hash of the trusted checkpoint block (must match --checkpoint-height).
        #[arg(long)]
        checkpoint_hash: Option<String>,
    },

    /// Initialize a new data directory
    Init {
        /// Network (mainnet, testnet)
        #[arg(long, default_value = "mainnet")]
        network: String,
    },

    /// Show node status
    Status,

    /// Import blocks from file
    Import {
        /// Path to blocks file
        path: PathBuf,
    },

    /// Export blocks to file
    Export {
        /// Path to output file
        path: PathBuf,

        /// Start height
        #[arg(long, default_value = "0")]
        from: u64,

        /// End height (default: latest)
        #[arg(long)]
        to: Option<u64>,
    },

    /// Update management commands
    Update {
        #[command(subcommand)]
        action: UpdateCommands,
    },

    /// Maintainer management commands
    Maintainer {
        #[command(subcommand)]
        action: MaintainerCommands,
    },

    /// Truncate the chain by removing the top N blocks.
    ///
    /// Rolls back state using undo data (up to 2000 blocks) and deletes
    /// blocks above the new tip. On restart, the node re-syncs from peers.
    /// Use for manual fork recovery on seed/archiver nodes.
    Truncate {
        /// Number of blocks to remove from the tip
        #[arg(long)]
        blocks: u64,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Recover chain state from existing block data
    ///
    /// Use this if nodes fail to load after ungraceful shutdown.
    /// Scans BlockStore for blocks and rebuilds UTXO set and chain state.
    Recover {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Restore chain from an archive directory (disaster recovery)
    ///
    /// Imports all blocks from the archive, verifies BLAKE3 checksums,
    /// then rebuilds UTXO/producer/chain state automatically.
    /// Use --backfill to only import missing blocks (fills snap sync gaps).
    Restore {
        /// Path to the archive directory (local files)
        #[arg(long, conflicts_with = "from_rpc")]
        from: Option<PathBuf>,

        /// RPC URL of an archiver node (e.g. http://archive.testnet.doli.network:18550)
        #[arg(long, conflicts_with = "from")]
        from_rpc: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,

        /// Only import missing blocks (skip existing). Does not rebuild state.
        #[arg(long)]
        backfill: bool,

        /// Skip genesis hash validation (only safe when backfilling from a trusted
        /// seed on the same chain). Required for post-reset chains where the embedded
        /// genesis hash differs from the current chain's genesis.
        #[arg(long, requires = "backfill")]
        skip_genesis_check: bool,
    },

    /// Rebuild canonical chain index from block headers
    ///
    /// Scans all headers in the block store (by hash, NOT by height_index),
    /// finds the true chain tip (highest slot), walks backwards via prev_hash
    /// to assign heights. Fixes corrupt height_index caused by fork blocks.
    /// Does NOT touch headers, bodies, UTXO, or producer data.
    Reindex,

    /// Local devnet management commands
    Devnet {
        #[command(subcommand)]
        action: DevnetCommands,
    },

    /// Release signing commands (for maintainers)
    Release {
        #[command(subcommand)]
        action: ReleaseCommands,
    },

    /// Upgrade to the latest release from GitHub
    ///
    /// Downloads a pre-built binary, verifies SHA256, and replaces the
    /// running binary via exec() — same PID, supervisor doesn't notice.
    Upgrade {
        /// Target version (default: latest)
        #[arg(long)]
        version: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Print checkpoint constants compiled into this binary.
    ///
    /// Used in the release workflow to verify checkpoint values
    /// before publishing a new binary.
    CheckpointInfo,
}

#[derive(Subcommand)]
pub(crate) enum UpdateCommands {
    /// Check for available updates
    Check,

    /// Show pending update status
    Status,

    /// Vote on a pending update (requires producer key)
    Vote {
        /// Vote to veto the update
        #[arg(long)]
        veto: bool,

        /// Vote to approve the update
        #[arg(long)]
        approve: bool,

        /// Path to producer key file
        #[arg(long)]
        key: PathBuf,
    },

    /// View current vote status for an update
    Votes {
        /// Version to check votes for (optional, uses pending update if not specified)
        #[arg(long)]
        version: Option<String>,
    },

    /// Apply a pending approved update
    Apply {
        /// Force apply even if not in enforcement period
        #[arg(long)]
        force: bool,
    },

    /// Rollback to previous version backup
    Rollback,

    /// Verify release signatures
    Verify {
        /// Version to verify
        #[arg(long)]
        version: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum MaintainerCommands {
    /// List current maintainer set
    List,

    /// Propose removing a maintainer (requires 3/5 multisig)
    Remove {
        /// Public key of maintainer to remove
        #[arg(long)]
        target: String,

        /// Path to maintainer key file for signing
        #[arg(long)]
        key: PathBuf,

        /// Reason for removal (optional)
        #[arg(long)]
        reason: Option<String>,
    },

    /// Propose adding a new maintainer (requires 3/5 multisig)
    Add {
        /// Public key of producer to add as maintainer
        #[arg(long)]
        target: String,

        /// Path to maintainer key file for signing
        #[arg(long)]
        key: PathBuf,
    },

    /// Sign a pending maintainer change proposal
    Sign {
        /// Proposal ID to sign
        #[arg(long)]
        proposal_id: String,

        /// Path to maintainer key file for signing
        #[arg(long)]
        key: PathBuf,
    },

    /// Verify if a public key is a maintainer
    Verify {
        /// Public key to check
        #[arg(long)]
        pubkey: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum DevnetCommands {
    /// Initialize a local devnet with N producer nodes
    Init {
        /// Number of nodes to create (1-20)
        #[arg(long, default_value = "3")]
        nodes: u32,
    },

    /// Start all devnet nodes
    Start,

    /// Stop all devnet nodes
    Stop,

    /// Show devnet status
    Status,

    /// Remove devnet data
    Clean {
        /// Keep wallet keys when cleaning
        #[arg(long)]
        keep_keys: bool,
    },

    /// Add producer(s) to a running devnet
    AddProducer {
        /// Number of producers to add
        #[arg(long, default_value = "1")]
        count: u32,
        /// Bonds per producer (default: 1)
        #[arg(long, short = 'b', default_value = "1")]
        bonds: u32,
        /// DOLI to fund each new wallet (default: auto = bonds + 1)
        #[arg(long)]
        fund_amount: Option<u64>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ReleaseCommands {
    /// Sign a release with a maintainer key
    ///
    /// Produces a JSON signature that can be fed into publish_release.sh.
    /// Signs "version:sha256" using Ed25519 — same format verified by nodes.
    Sign {
        /// Path to maintainer/producer key file (JSON wallet format)
        #[arg(long)]
        key: PathBuf,

        /// Version tag (e.g., v0.2.0)
        #[arg(long)]
        version: String,

        /// SHA-256 hash of the canonical binary (linux-x64-musl).
        /// If omitted, fetches CHECKSUMS.txt from the GitHub release.
        #[arg(long)]
        hash: Option<String>,
    },
}
