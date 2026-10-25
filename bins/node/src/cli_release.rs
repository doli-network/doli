//! `doli-node release` subcommands.

use std::path::PathBuf;

use clap::Subcommand;

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
