//! `exited-bond-census`: offline, read-only list of the Exited producers of a
//! stopped node that still own Bond UTXOs. Uses the `check-producer-set`
//! loader (lock probe, scratch dir, read-only `state_db`); nothing is written
//! to the data dir.

use std::io::Write;
use std::path::Path;

use anyhow::{bail, Result};
use doli_core::transaction::OutputType;
use doli_core::validation::bond_owner;
use serde::Serialize;
use storage::ProducerStatus;

use super::producer_set_check::open_stopped;

#[derive(Serialize)]
struct BondRecord {
    outpoint: String,
    amount: u64,
}

#[derive(Serialize)]
struct ProducerRecord {
    key: String,
    status: &'static str,
    bond_count: usize,
    total: u64,
    bonds: Vec<BondRecord>,
}

/// Prints one JSON line per Exited producer owning at least one Bond UTXO
/// (sorted by key), then `height= exited_with_bonds= bond_utxos= total=`.
/// A held `state_db` or `blocks` LOCK is an error.
pub(crate) fn exited_bond_census(data_dir: &Path) -> Result<()> {
    let Some(st) = open_stopped(data_dir)? else {
        bail!("lock held on the store by another process (stop the node first)");
    };
    let ps = st.state_db.load_producer_set();
    let mut exited: Vec<_> = ps
        .all_producers()
        .into_iter()
        .filter(|p| p.status == ProducerStatus::Exited)
        .map(|p| p.public_key)
        .collect();
    exited.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    let mut out = std::io::stdout().lock();
    let (mut producers, mut bond_utxos, mut total) = (0usize, 0usize, 0u64);
    for key in exited {
        let bonds: Vec<BondRecord> = st
            .state_db
            .get_utxos_by_pubkey(&bond_owner(&key))
            .into_iter()
            .filter(|(_, e)| e.output.output_type == OutputType::Bond)
            .map(|(op, e)| BondRecord {
                outpoint: format!("{}:{}", op.tx_hash.to_hex(), op.index),
                amount: e.output.amount,
            })
            .collect();
        if bonds.is_empty() {
            continue;
        }
        let sum: u64 = bonds.iter().map(|b| b.amount).sum();
        producers += 1;
        bond_utxos += bonds.len();
        total += sum;
        let record = ProducerRecord {
            key: key.to_hex(),
            status: "Exited",
            bond_count: bonds.len(),
            total: sum,
            bonds,
        };
        writeln!(out, "{}", serde_json::to_string(&record)?)?;
    }
    writeln!(
        out,
        "height={} exited_with_bonds={producers} bond_utxos={bond_utxos} total={total}",
        st.cs.best_height
    )?;
    out.flush()?;
    Ok(())
}
