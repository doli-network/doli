use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "doli")]
#[command(about = "DOLI wallet CLI", long_about = None)]
#[command(version = env!("DOLI_VERSION_STRING"))]
pub(crate) struct Cli {
    /// Wallet file path (default: auto-detected from network)
    #[arg(short, long)]
    pub(crate) wallet: Option<String>,

    /// Node RPC endpoint (auto-detected from --network if not set)
    #[arg(short, long, env = "DOLI_RPC_URL")]
    pub(crate) rpc: Option<String>,

    /// Network (mainnet, testnet, devnet)
    #[arg(short, long, default_value = "mainnet", env = "DOLI_NETWORK")]
    pub(crate) network: String,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Commands {
    /// Initialize a new producer wallet (combines 'new' + 'add-bls')
    Init {
        /// Overwrite existing wallet (DANGEROUS: destroys existing keys)
        #[arg(long)]
        force: bool,

        /// Skip BLS key generation (non-producer wallet)
        #[arg(long)]
        non_producer: bool,
    },

    /// Create a new wallet
    New {
        /// Wallet name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Restore a wallet from a 24-word seed phrase
    Restore {
        /// Wallet name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Generate a new address
    Address {
        /// Label for the address
        #[arg(short, long)]
        label: Option<String>,
    },

    /// List all addresses
    Addresses,

    /// Show wallet balance
    Balance {
        /// Show balance for a specific address only
        #[arg(short = 'A', long)]
        address: Option<String>,

        /// Show per-address breakdown (all addresses in wallet)
        #[arg(long)]
        all: bool,
    },

    /// Send coins (optionally with a covenant condition)
    Send {
        /// Recipient address
        to: String,

        /// Amount to send
        amount: String,

        /// Fee (default: auto)
        #[arg(short, long)]
        fee: Option<String>,

        /// Covenant condition on the output. Examples:
        ///   multisig(2, addr1, addr2, addr3)
        ///   hashlock(hex_hash)
        ///   htlc(hex_hash, lock_height, expiry_height)
        ///   timelock(min_height)
        ///   vesting(addr, unlock_height)
        #[arg(short, long)]
        condition: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Spend a covenant-conditioned UTXO
    Spend {
        /// UTXO to spend: txhash:output_index
        utxo: String,

        /// Recipient address
        to: String,

        /// Amount to send (remaining goes to change)
        amount: String,

        /// Witness data to satisfy the condition. Examples:
        ///   preimage(hex_secret)
        ///   sign(wallet1.json, wallet2.json)
        ///   branch(right, preimage(hex_secret))
        #[arg(short, long)]
        witness: String,

        /// Fee (default: auto)
        #[arg(short, long)]
        fee: Option<String>,
    },

    /// Show transaction history
    History {
        /// Maximum number of transactions
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Export wallet
    Export {
        /// Output file path
        output: PathBuf,
    },

    /// Import wallet
    Import {
        /// Input file path
        input: PathBuf,
    },

    /// Show wallet info
    Info,

    /// Add BLS attestation key to an existing wallet
    AddBls,

    /// Sign a message
    Sign {
        /// Message to sign
        message: String,

        /// Address to sign with
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Verify a signature
    Verify {
        /// Message that was signed
        message: String,

        /// Signature (hex)
        signature: String,

        /// Public key or address
        pubkey: String,
    },

    /// Producer commands
    Producer {
        #[command(subcommand)]
        command: ProducerCommands,
    },

    /// Rewards commands (epoch presence rewards)
    Rewards {
        #[command(subcommand)]
        command: RewardsCommands,
    },

    /// Show chain information
    Chain,

    /// Verify chain integrity and compute chain commitment
    ///
    /// Scans all blocks 1..tip and computes a running BLAKE3 commitment:
    ///   commitment[N] = BLAKE3(commitment[N-1] || block_hash[N])
    /// Two nodes with the same commitment have identical chains.
    ChainVerify,

    /// Update governance commands
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },

    /// Maintainer governance commands
    Maintainer {
        #[command(subcommand)]
        command: MaintainerCommands,
    },

    /// Upgrade doli binaries to the latest release
    Upgrade {
        /// Target version (default: latest)
        #[arg(long)]
        version: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,

        /// Custom path to doli-node binary (skip auto-detection)
        #[arg(long)]
        doli_node_path: Option<std::path::PathBuf>,

        /// Restart only this systemd service (e.g. doli-mainnet-node3)
        #[arg(long)]
        service: Option<String>,
    },

    /// Release management commands (maintainer signing)
    Release {
        #[command(subcommand)]
        command: ReleaseCommands,
    },

    /// Protocol activation commands (on-chain consensus upgrades)
    Protocol {
        #[command(subcommand)]
        command: ProtocolCommands,
    },

    /// NFT operations (mint, transfer, buy, sell, list, info)
    Nft {
        /// List NFTs owned by this wallet
        #[arg(short, long)]
        list: bool,

        /// Show info for a specific NFT UTXO (txhash:index)
        #[arg(short, long, value_name = "UTXO")]
        info: Option<String>,

        /// Mint a new NFT with given content (hash, URI, or text)
        #[arg(short, long, value_name = "CONTENT")]
        mint: Option<String>,

        /// Transfer an NFT to a new owner (requires --to)
        #[arg(short, long, value_name = "UTXO")]
        transfer: Option<String>,

        /// Create a sell offer file (requires --price, -o)
        #[arg(short, long, value_name = "UTXO")]
        sell: Option<String>,

        /// Create a SIGNED sell offer (PSBT). Seller signs their NFT input, buyer completes later.
        /// No --seller-wallet needed on buy side. Requires --price, --to (buyer address), -o.
        #[arg(long, value_name = "UTXO")]
        sell_sign: Option<String>,

        /// Buy an NFT directly (requires --price, --seller-wallet)
        #[arg(short, long, value_name = "UTXO")]
        buy: Option<String>,

        /// Buy from a sell offer file. If offer is signed (PSBT), no --seller-wallet needed.
        #[arg(long, value_name = "FILE")]
        from: Option<String>,

        /// Recipient address (for --transfer or --sell-sign)
        #[arg(long)]
        to: Option<String>,

        /// Price in DOLI (for --sell, --sell-sign, or --buy)
        #[arg(long)]
        price: Option<String>,

        /// Output file for sell offer (for --sell or --sell-sign)
        #[arg(short = 'o', long = "out", value_name = "FILE")]
        output: Option<String>,

        /// Seller's wallet path (for --buy or unsigned --from)
        #[arg(long)]
        seller_wallet: Option<String>,

        /// Condition on the NFT (for --mint, default: signature)
        #[arg(short, long)]
        condition: Option<String>,

        /// DOLI value to attach to NFT (for --mint, default: 0)
        #[arg(short, long, default_value = "0")]
        amount: String,

        /// Witness to satisfy spending condition (for --transfer)
        #[arg(short, long, default_value = "none()")]
        witness: String,

        /// Royalty in percent for the creator (for --mint, e.g. 5 = 5%)
        #[arg(long)]
        royalty: Option<f64>,

        /// Raw binary data (hex-encoded) to embed in the NFT on-chain (for --mint)
        #[arg(long, value_name = "HEX")]
        data: Option<String>,

        /// Export NFT content to a file (extract from on-chain UTXO)
        #[arg(long, value_name = "UTXO")]
        export: Option<String>,

        /// Batch mint NFTs from a JSON manifest file
        #[arg(long, value_name = "FILE")]
        batch_mint: Option<String>,

        /// Skip confirmation prompt (for --batch-mint)
        #[arg(long)]
        yes: bool,
    },

    /// Issue a fungible token (meme coin, stablecoin, etc.)
    IssueToken {
        /// Token ticker (e.g. DOGEOLI, max 16 chars)
        ticker: String,

        /// Total supply (fixed at issuance, in token units)
        #[arg(long)]
        supply: u64,

        /// Optional condition on spending (default: signature of issuer)
        #[arg(short, long)]
        condition: Option<String>,
    },

    /// Show token info from a UTXO
    TokenInfo {
        /// UTXO containing the fungible asset: txhash:output_index
        utxo: String,
    },

    /// Initiate a complete cross-chain atomic swap (generates preimage, locks DOLI)
    BridgeSwap {
        /// Amount of DOLI to lock
        amount: String,

        /// Target chain: bitcoin, ethereum, monero, litecoin, cardano, bsc
        #[arg(long)]
        chain: String,

        /// Recipient address on the target chain
        #[arg(long)]
        to: String,

        /// Counter-chain RPC endpoint (e.g. http://localhost:8332 for Bitcoin)
        #[arg(long)]
        counter_rpc: Option<String>,

        /// Required confirmations on counter-chain (default: 6 BTC, 12 ETH)
        #[arg(long, default_value = "0")]
        confirmations: u32,
    },

    /// Check the status of a bridge swap (reads on-chain state, no watcher needed)
    BridgeStatus {
        /// Swap ID: txhash:output_index
        swap_id: String,

        /// Bitcoin Core RPC endpoint for counter-chain monitoring
        #[arg(long)]
        btc_rpc: Option<String>,

        /// Ethereum RPC endpoint for counter-chain monitoring
        #[arg(long)]
        eth_rpc: Option<String>,

        /// Auto-claim/refund when conditions are met
        #[arg(long)]
        auto: bool,
    },

    /// Buy into an existing bridge swap (counterparty/buyer side)
    BridgeBuy {
        /// Swap ID: txhash:output_index (the BridgeHTLC UTXO on DOLI)
        swap_id: String,

        /// Preimage (64 hex chars) — if you already have it from the counter-chain
        #[arg(long)]
        preimage: Option<String>,

        /// Bitcoin Core RPC endpoint (to auto-detect preimage from BTC claim)
        #[arg(long)]
        btc_rpc: Option<String>,

        /// Ethereum/BSC RPC endpoint (to auto-detect preimage from ETH claim)
        #[arg(long)]
        eth_rpc: Option<String>,

        /// Send claimed DOLI to this address instead of your wallet
        #[arg(long)]
        to: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// List active bridge HTLC swaps on-chain
    BridgeList {
        /// Filter by target chain (bitcoin, ethereum, bsc, monero, etc.)
        #[arg(long)]
        chain: Option<String>,

        /// Number of recent blocks to scan (default: 100)
        #[arg(long, default_value = "100")]
        blocks: u64,
    },

    /// Lock DOLI in a bridge HTLC for cross-chain atomic swap (manual, advanced)
    BridgeLock {
        /// Amount of DOLI to lock
        amount: String,

        /// BLAKE3 hashlock hash (64 hex chars) — use this OR --preimage, not both
        #[arg(long, conflicts_with = "preimage")]
        hash: Option<String>,

        /// Preimage (64 hex chars) — computes the hashlock automatically
        #[arg(long, conflicts_with = "hash")]
        preimage: Option<String>,

        /// Lock height (claim available after this)
        #[arg(long)]
        lock: u64,

        /// Expiry height (refund available after this)
        #[arg(long)]
        expiry: u64,

        /// Target chain: bitcoin, ethereum, monero, litecoin, cardano, bsc
        #[arg(long)]
        chain: String,

        /// Recipient address on the target chain
        #[arg(long)]
        to: String,

        /// Counter-chain hash (SHA256 for Bitcoin, keccak256 for Ethereum). 64 hex chars.
        #[arg(long)]
        counter_hash: String,

        /// Multisig threshold (requires --multisig-keys)
        #[arg(long)]
        multisig_threshold: Option<u8>,

        /// Comma-separated multisig key addresses (requires --multisig-threshold)
        #[arg(long)]
        multisig_keys: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Claim a bridge HTLC with the preimage (receiver side)
    BridgeClaim {
        /// UTXO containing the bridge HTLC: txhash:output_index
        utxo: String,

        /// Preimage (64 hex chars) that hashes to the locked hash
        #[arg(long)]
        preimage: String,

        /// Send claimed funds to this address instead of your wallet
        #[arg(long)]
        to: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Refund a bridge HTLC after expiry (sender side)
    BridgeRefund {
        /// UTXO containing the bridge HTLC: txhash:output_index
        utxo: String,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Pool AMM operations (create, swap, add/remove liquidity)
    Pool {
        #[command(subcommand)]
        command: PoolCommands,
    },

    /// Lending operations (deposit, withdraw, borrow, repay, liquidate)
    Loan {
        #[command(subcommand)]
        command: LoanCommands,
    },

    /// Payment channel operations (open, pay, close, list, info)
    Channel {
        #[command(subcommand)]
        command: ChannelCommands,
    },

    /// Manage the doli-node system service
    #[command(subcommand)]
    Service(ServiceCommand),

    /// Seed Guardian commands (halt, resume, checkpoint, fork monitor)
    Guardian {
        #[command(subcommand)]
        command: GuardianCommands,
    },

    /// Fast-sync: wipe chain data and download a verified state snapshot from the network
    Snap {
        /// Data directory (for multi-node servers with custom paths)
        #[arg(short, long)]
        data_dir: Option<PathBuf>,

        /// Custom seed RPC endpoint(s) to use instead of network defaults.
        /// Can be specified multiple times: --seed http://127.0.0.1:8500 --seed http://other:8500
        #[arg(long)]
        seed: Vec<String>,

        /// Skip service stop/restart (for manual or multi-node local setups)
        #[arg(long)]
        no_restart: bool,

        /// Trust a single seed without consensus verification (useful for local/dev)
        #[arg(long)]
        trust: bool,
    },

    /// Wipe chain data for a fresh resync (preserves keys/ and .env)
    Wipe {
        /// Network (mainnet, testnet, devnet)
        #[arg(short, long, default_value = "mainnet")]
        network: String,

        /// Data directory (overrides network default)
        #[arg(short, long)]
        data_dir: Option<PathBuf>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum PoolCommands {
    /// Create a new AMM pool with initial liquidity
    Create {
        /// FungibleAsset ID (hex) for the token side of the pool
        #[arg(long)]
        asset: String,

        /// Initial DOLI amount
        #[arg(long)]
        doli: String,

        /// Initial token amount (raw token units, NOT DOLI notation)
        #[arg(long)]
        tokens: String,

        /// Fee in basis points (default: 30 = 0.3%)
        #[arg(long, default_value = "30")]
        fee: u16,

        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },

    /// Swap assets through a pool
    Swap {
        /// Pool ID (hex)
        #[arg(long)]
        pool: String,

        /// Amount to swap (DOLI notation for a2b, raw token units for b2a)
        #[arg(long)]
        amount: String,

        /// Direction: a2b (DOLI->token) or b2a (token->DOLI)
        #[arg(long)]
        direction: String,

        /// Minimum output (raw token units for a2b, DOLI notation for b2a)
        #[arg(long)]
        min_out: Option<String>,

        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },

    /// Add liquidity to a pool
    Add {
        /// Pool ID (hex)
        #[arg(long)]
        pool: String,

        /// DOLI amount to add
        #[arg(long)]
        doli: String,

        /// Token amount to add (raw token units, NOT DOLI notation)
        #[arg(long)]
        tokens: String,

        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },

    /// Remove liquidity from a pool (burn LP shares)
    Remove {
        /// Pool ID (hex)
        #[arg(long)]
        pool: String,

        /// LP shares to burn (raw units)
        #[arg(long)]
        shares: String,

        /// Minimum DOLI to receive (DOLI notation)
        #[arg(long)]
        min_doli: Option<String>,

        /// Minimum tokens to receive (raw token units)
        #[arg(long)]
        min_tokens: Option<String>,

        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },

    /// List all pools
    List,

    /// Show pool info
    Info {
        /// Pool ID (hex)
        pool_id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum LoanCommands {
    /// Deposit DOLI into lending pool to earn interest
    Deposit {
        /// Pool ID (hex)
        #[arg(long)]
        pool: String,
        /// Amount of DOLI to deposit
        #[arg(long)]
        amount: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Withdraw DOLI + earned interest from lending pool
    Withdraw {
        /// LendingDeposit UTXO: txhash:output_index
        deposit_utxo: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Create a collateralized loan (borrow DOLI against tokens)
    Create {
        /// Pool ID (hex)
        #[arg(long)]
        pool: String,
        /// Collateral amount (token units)
        #[arg(long)]
        collateral: String,
        /// Amount of DOLI to borrow
        #[arg(long)]
        borrow: String,
        /// Interest rate in basis points (default: 500 = 5%)
        #[arg(long, default_value = "500")]
        interest_rate: u16,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Repay loan and recover collateral
    Repay {
        /// Collateral UTXO: txhash:output_index
        loan_utxo: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Liquidate undercollateralized loan
    Liquidate {
        /// Collateral UTXO: txhash:output_index
        loan_utxo: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
    },
    /// List active loans
    List {
        /// Filter by borrower address (hex)
        #[arg(long)]
        borrower: Option<String>,
    },
    /// Show loan details
    Info {
        /// Collateral UTXO: txhash:output_index
        loan_utxo: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChannelCommands {
    /// Open a payment channel with a counterparty
    Open {
        /// Counterparty's address (doli1... or hex pubkey_hash)
        peer: String,

        /// Channel capacity in DOLI
        capacity: String,

        /// Fee for the funding transaction (default: auto)
        #[arg(short, long)]
        fee: Option<String>,
    },

    /// Send a payment through a channel
    Pay {
        /// Channel ID (hex, first 16 chars sufficient)
        channel: String,

        /// Amount to send in DOLI
        amount: String,
    },

    /// Cooperatively close a channel
    Close {
        /// Channel ID (hex)
        channel: String,

        /// Fee for the close transaction (default: auto)
        #[arg(short, long)]
        fee: Option<String>,

        /// Force close (unilateral, uses latest commitment tx)
        #[arg(long)]
        force: bool,
    },

    /// List all channels
    List {
        /// Show all channels including closed
        #[arg(short, long)]
        all: bool,
    },

    /// Show detailed channel info
    Info {
        /// Channel ID (hex)
        channel: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProducerCommands {
    /// Register as a block producer
    Register {
        /// Number of bonds to stake (1-10000, each bond = 1 bond_unit)
        #[arg(short, long, default_value = "1")]
        bonds: u32,
    },

    /// Check producer status
    Status {
        /// Public key (optional, uses wallet if not specified)
        #[arg(short, long)]
        pubkey: Option<String>,
    },

    /// Show per-bond vesting details
    Bonds {
        /// Public key (optional, uses wallet if not specified)
        #[arg(short, long)]
        pubkey: Option<String>,
    },

    /// List all producers in the network
    List {
        /// Show only active producers
        #[arg(short, long)]
        active: bool,

        /// Output format: table (default), json, csv
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Add more bonds to increase stake (bond stacking)
    AddBond {
        /// Number of bonds to add (1-10000)
        #[arg(short, long)]
        count: u32,
    },

    /// Withdraw bonds instantly (FIFO, vesting penalty applies)
    RequestWithdrawal {
        /// Number of bonds to withdraw
        #[arg(short, long)]
        count: u32,

        /// Destination address for withdrawn funds (hex pubkey_hash)
        #[arg(short, long)]
        destination: Option<String>,
    },

    /// Simulate bond withdrawal (dry run, no transaction)
    SimulateWithdrawal {
        /// Number of bonds to simulate withdrawing
        #[arg(short, long)]
        count: u32,
    },

    /// Exit the producer set (early exit incurs penalty)
    Exit {
        /// Force early exit with penalty
        #[arg(long)]
        force: bool,
    },

    /// Submit slashing evidence for double production (equivocation)
    Slash {
        /// Block 1 hash (first conflicting block)
        #[arg(long)]
        block1: String,

        /// Block 2 hash (second conflicting block for same slot)
        #[arg(long)]
        block2: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum RewardsCommands {
    /// List all claimable epochs with estimated rewards
    List,

    /// Claim rewards for a specific epoch
    Claim {
        /// Epoch number to claim
        epoch: u64,

        /// Recipient address (defaults to wallet address)
        #[arg(short, long)]
        recipient: Option<String>,
    },

    /// Claim all available rewards (one tx per epoch)
    ClaimAll {
        /// Recipient address (defaults to wallet address)
        #[arg(short, long)]
        recipient: Option<String>,
    },

    /// Show claim history
    History {
        /// Maximum number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Show current epoch info and BLOCKS_PER_REWARD_EPOCH
    Info,
}

#[derive(Subcommand)]
pub(crate) enum UpdateCommands {
    /// Check for available updates
    Check,

    /// Show pending update status, veto progress, deadlines
    Status,

    /// Vote on a pending update
    Vote {
        /// Version to vote on (e.g., "1.0.1")
        #[arg(long)]
        version: String,

        /// Cast a veto vote
        #[arg(long, conflicts_with = "approve")]
        veto: bool,

        /// Cast an approval vote
        #[arg(long, conflicts_with = "veto")]
        approve: bool,
    },

    /// Show current votes for a version
    Votes {
        /// Version to check (e.g., "1.0.1")
        #[arg(long)]
        version: String,
    },

    /// Manually apply an approved update
    Apply,

    /// Rollback to previous version
    Rollback,
}

#[derive(Subcommand)]
pub(crate) enum MaintainerCommands {
    /// List current maintainers
    List,
}

#[derive(Subcommand)]
pub(crate) enum ReleaseCommands {
    /// Sign a release (maintainer workflow)
    ///
    /// Signs the message "{version}:{sha256(CHECKSUMS.txt)}" with a producer key.
    /// Output is a JSON signature block for inclusion in SIGNATURES.json.
    Sign {
        /// Release version to sign (e.g., v1.0.27)
        #[arg(long)]
        version: String,

        /// Path to producer key file (overrides -w wallet)
        #[arg(long)]
        key: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProtocolCommands {
    /// Sign a protocol activation (one maintainer at a time)
    ///
    /// Signs "activate:{version}:{epoch}" with a producer key.
    /// Output is a JSON signature block. Collect 3/5 and pass to `protocol activate`.
    Sign {
        /// Protocol version to activate (e.g., 2)
        #[arg(long)]
        version: u32,

        /// Epoch at which activation occurs
        #[arg(long)]
        epoch: u64,

        /// Path to producer key file (overrides -w wallet)
        #[arg(long)]
        key: Option<PathBuf>,
    },

    /// Submit a protocol activation transaction (requires 3/5 signatures)
    ///
    /// Reads signature blocks from a JSON file, builds the ProtocolActivation tx,
    /// and submits it to the network.
    Activate {
        /// Protocol version to activate (e.g., 2)
        #[arg(long)]
        version: u32,

        /// Epoch at which activation occurs
        #[arg(long)]
        epoch: u64,

        /// Description of consensus changes
        #[arg(long)]
        description: String,

        /// Path to JSON file containing collected signatures
        #[arg(long)]
        signatures: PathBuf,
    },
}

#[derive(Subcommand)]
pub(crate) enum GuardianCommands {
    /// Show guardian status (production state, checkpoints, chain tip)
    Status,

    /// Pause block production on this node (emergency halt)
    Halt {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Resume block production (clear emergency halt)
    Resume,

    /// Create a RocksDB checkpoint (hot backup)
    Checkpoint,

    /// Monitor multiple nodes for chain forks
    Monitor {
        /// Node RPC endpoint(s) to monitor (repeat for each node)
        #[arg(long = "endpoint", required = true)]
        endpoints: Vec<String>,

        /// Continuous monitoring interval in seconds (omit for single check)
        #[arg(long = "loop")]
        loop_secs: Option<u64>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ServiceCommand {
    /// Install and start the node service (requires sudo on Linux)
    Install {
        /// Network (mainnet, testnet, devnet)
        #[arg(short, long, default_value = "mainnet")]
        network: String,

        /// Custom service name (default: doli-{network})
        #[arg(long)]
        name: Option<String>,

        /// Data directory (default: /var/lib/doli/{network} on Linux, ~/Library/Application Support/doli/{network} on macOS)
        #[arg(long)]
        data_dir: Option<String>,

        /// Path to producer key file
        #[arg(long)]
        producer_key: Option<String>,

        /// P2P listen port
        #[arg(long)]
        p2p_port: Option<u16>,

        /// RPC listen port
        #[arg(long)]
        rpc_port: Option<u16>,
    },

    /// Remove the node service
    Uninstall {
        /// Custom service name
        #[arg(long)]
        name: Option<String>,
    },

    /// Start the node service
    Start {
        /// Custom service name
        #[arg(long)]
        name: Option<String>,
    },

    /// Stop the node service
    Stop {
        /// Custom service name
        #[arg(long)]
        name: Option<String>,
    },

    /// Restart the node service
    Restart {
        /// Custom service name
        #[arg(long)]
        name: Option<String>,
    },

    /// Show node service status
    Status {
        /// Custom service name
        #[arg(long)]
        name: Option<String>,
    },

    /// Show node logs
    Logs {
        /// Custom service name
        #[arg(long)]
        name: Option<String>,

        /// Follow log output (like tail -f)
        #[arg(short, long)]
        follow: bool,

        /// Number of lines to show
        #[arg(short = 'n', long, default_value = "50")]
        lines: u32,
    },
}
