//! Read-only verification tool for BUG-001 (Pool stamping divergence).
//!
//! Opens BOTH the state_db and utxo_store of a running node in read-only
//! (secondary) mode, finds every Pool UTXO, and diffs the bytes between
//! the two stores. Reports any divergence.
//!
//! Usage: cargo run --release --example pool_byte_diff -- <node-data-dir>
//!
//! The data dir must contain `state_db/` and `utxo_store/` subdirectories.

use rocksdb::{ColumnFamilyDescriptor, IteratorMode, Options, DB};
use storage::UtxoEntry;

// cf_unique_id is added in Phase 1 of the redesign (specs/utxo-storage-architecture.md)
// and is not present on pre-redesign databases. Omitting it here keeps this binary
// compatible with currently-deployed nodes.
const STATE_DB_CFS: &[&str] = &[
    "cf_utxo",
    "cf_utxo_by_pubkey",
    "cf_producers",
    "cf_exit_history",
    "cf_meta",
    "cf_undo",
];
const UTXO_STORE_CFS: &[&str] = &["utxo", "utxo_by_pubkey", "unique_id"];

const STATE_DB_CF_UTXO: &str = "cf_utxo";
const UTXO_STORE_CF_UTXO: &str = "utxo";

fn open_secondary(path: &str, cfs: &[&str]) -> DB {
    let mut opts = Options::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);

    let cf_descriptors: Vec<ColumnFamilyDescriptor> = cfs
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()))
        .collect();

    // Secondary instance: read-only, does not interfere with the primary.
    let secondary_path = format!("{}_secondary_pool_diff", path);
    let _ = std::fs::create_dir_all(&secondary_path);
    DB::open_cf_descriptors_as_secondary(&opts, path, &secondary_path, cf_descriptors)
        .unwrap_or_else(|e| panic!("Failed to open {} as secondary: {}", path, e))
}

fn main() {
    let data_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| panic!("Usage: pool_byte_diff <node-data-dir>"));

    let state_db_path = format!("{}/state_db", data_dir);
    let utxo_store_path = format!("{}/utxo_store", data_dir);

    eprintln!("Opening state_db at: {}", state_db_path);
    let state_db = open_secondary(&state_db_path, STATE_DB_CFS);
    eprintln!("Opening utxo_store at: {}", utxo_store_path);
    let utxo_store = open_secondary(&utxo_store_path, UTXO_STORE_CFS);

    // Catch up secondaries to the latest WAL state.
    let _ = state_db.try_catch_up_with_primary();
    let _ = utxo_store.try_catch_up_with_primary();

    let state_cf = state_db
        .cf_handle(STATE_DB_CF_UTXO)
        .expect("state_db missing cf_utxo");
    let utxo_cf = utxo_store
        .cf_handle(UTXO_STORE_CF_UTXO)
        .expect("utxo_store missing utxo");

    let mut total_utxos_state = 0u64;
    let mut total_utxos_store = 0u64;
    let mut pools_state = 0u64;
    let mut pools_store = 0u64;
    let mut pools_compared = 0u64;
    let mut pools_match = 0u64;
    let mut pools_diff = 0u64;
    let mut first_diff_printed = false;

    // Iterate state_db.cf_utxo, find Pools, compare each against utxo_store.utxo.
    for item in state_db.iterator_cf(state_cf, IteratorMode::Start) {
        let (key, state_bytes) = match item {
            Ok(kv) => kv,
            Err(e) => {
                eprintln!("state_db iterator error: {}", e);
                continue;
            }
        };
        total_utxos_state += 1;

        let entry_state: UtxoEntry = match bincode::deserialize(&state_bytes) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry_state.output.output_type != doli_core::OutputType::Pool {
            continue;
        }
        pools_state += 1;

        // Look up the same outpoint in utxo_store.
        let store_bytes = match utxo_store.get_cf(utxo_cf, &key) {
            Ok(Some(b)) => b,
            Ok(None) => {
                eprintln!(
                    "MISSING in utxo_store: outpoint {} (Pool present in state_db only)",
                    hex_short(&key)
                );
                continue;
            }
            Err(e) => {
                eprintln!("utxo_store get_cf error: {}", e);
                continue;
            }
        };

        pools_compared += 1;

        if state_bytes.as_ref() == store_bytes.as_slice() {
            pools_match += 1;
        } else {
            pools_diff += 1;
            if !first_diff_printed {
                first_diff_printed = true;
                eprintln!("\n=== DIVERGENCE at outpoint {} ===", hex_short(&key));
                eprintln!(
                    "state_db bytes ({} bytes): {}",
                    state_bytes.len(),
                    hex_short(&state_bytes)
                );
                eprintln!(
                    "utxo_store bytes ({} bytes): {}",
                    store_bytes.len(),
                    hex_short(&store_bytes)
                );
                let diff_offsets = find_diff_offsets(&state_bytes, &store_bytes);
                eprintln!(
                    "Divergent offsets (first 20): {:?}",
                    &diff_offsets[..diff_offsets.len().min(20)]
                );

                // Decode and report Pool metadata fields from each.
                if let Some(meta) = entry_state.output.pool_metadata() {
                    eprintln!(
                        "state_db pool_metadata: creation_slot={} last_update_slot={} cumulative_price={} reserve_a={} reserve_b={}",
                        meta.creation_slot, meta.last_update_slot, meta.cumulative_price, meta.reserve_a, meta.reserve_b
                    );
                }
                if let Ok(entry_store) = bincode::deserialize::<UtxoEntry>(&store_bytes) {
                    if let Some(meta) = entry_store.output.pool_metadata() {
                        eprintln!(
                            "utxo_store pool_metadata: creation_slot={} last_update_slot={} cumulative_price={} reserve_a={} reserve_b={}",
                            meta.creation_slot, meta.last_update_slot, meta.cumulative_price, meta.reserve_a, meta.reserve_b
                        );
                    }
                }
            }
        }
    }

    // Count Pools in utxo_store independently for cross-check.
    for item in utxo_store.iterator_cf(utxo_cf, IteratorMode::Start) {
        let (_, value) = match item {
            Ok(kv) => kv,
            Err(_) => continue,
        };
        total_utxos_store += 1;
        if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
            if entry.output.output_type == doli_core::OutputType::Pool {
                pools_store += 1;
            }
        }
    }

    println!("\n=== SUMMARY ===");
    println!("Total UTXOs in state_db.cf_utxo:    {}", total_utxos_state);
    println!("Total UTXOs in utxo_store.utxo:     {}", total_utxos_store);
    println!("Pools in state_db.cf_utxo:          {}", pools_state);
    println!("Pools in utxo_store.utxo:           {}", pools_store);
    println!("Pools compared:                     {}", pools_compared);
    println!("Pools BYTE-IDENTICAL:               {}", pools_match);
    println!("Pools DIVERGENT:                    {}", pools_diff);

    if pools_diff == 0 && pools_compared > 0 {
        println!("\nVERDICT: NO BUG. Both stores hold identical Pool bytes.");
    } else if pools_diff > 0 {
        println!(
            "\nVERDICT: BUG-001 CONFIRMED. {} Pool UTXOs differ between stores.",
            pools_diff
        );
    } else {
        println!("\nVERDICT: No Pools to compare on this node.");
    }
}

fn hex_short(bytes: &[u8]) -> String {
    if bytes.len() <= 64 {
        bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    } else {
        let head: String = bytes[..32].iter().map(|b| format!("{:02x}", b)).collect();
        let tail: String = bytes[bytes.len() - 32..]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        format!("{}...{}", head, tail)
    }
}

fn find_diff_offsets(a: &[u8], b: &[u8]) -> Vec<usize> {
    let mut diffs = Vec::new();
    let max_len = a.len().max(b.len());
    for i in 0..max_len {
        let av = a.get(i).copied();
        let bv = b.get(i).copied();
        if av != bv {
            diffs.push(i);
            if diffs.len() >= 40 {
                break;
            }
        }
    }
    diffs
}
