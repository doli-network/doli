//! Measurement tool: deserialize cf_undo entries and print byte-level breakdown.
//!
//! Usage: cargo run --release --example undo_size_breakdown -- <path-to-state_db>
//!
//! Opens the state_db in read-only mode, iterates all cf_undo entries, and prints
//! the size of each component: producer_snapshot, epoch_state_snapshot, spent_utxos,
//! created_utxos. Then decomposes ProducerSet and EpochState to find the bloat source.

use storage::UndoData;

const CF_UNDO: &str = "cf_undo";

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| panic!("Usage: undo_size_breakdown <path-to-state_db>"));

    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);

    let cfs = vec![
        "cf_utxo",
        "cf_utxo_by_pubkey",
        "cf_producers",
        "cf_exit_history",
        "cf_meta",
        CF_UNDO,
    ];

    let db = rocksdb::DB::open_cf_for_read_only(&opts, &path, &cfs, false)
        .unwrap_or_else(|e| panic!("Failed to open state_db at {path}: {e}"));

    let cf = db.cf_handle(CF_UNDO).expect("cf_undo not found");

    println!("=== cf_undo Size Breakdown ===");
    println!();

    let mut count = 0u64;
    let mut total_raw = 0u64;
    let mut total_producer_snap = 0u64;
    let mut total_epoch_snap = 0u64;
    let mut total_spent_utxos_size = 0u64;
    let mut total_created_utxos_size = 0u64;

    // Collect first 5 + last 5 entries for detailed breakdown
    let mut first_samples: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut last_sample: Option<(u64, Vec<u8>)> = None;
    let mut min_height = u64::MAX;
    let mut max_height = 0u64;

    for result in db.iterator_cf(cf, rocksdb::IteratorMode::Start).flatten() {
        let (key, value) = result;
        if key.len() != 8 {
            continue;
        }
        let height = u64::from_le_bytes(key[..8].try_into().unwrap());
        let raw_size = value.len() as u64;
        total_raw += raw_size;
        count += 1;
        min_height = min_height.min(height);
        max_height = max_height.max(height);

        match bincode::deserialize::<UndoData>(&value) {
            Ok(undo) => {
                total_producer_snap += undo.producer_snapshot.len() as u64;
                total_epoch_snap += undo
                    .epoch_state_snapshot
                    .as_ref()
                    .map(|s| s.len() as u64)
                    .unwrap_or(0);
                total_spent_utxos_size += bincode::serialized_size(&undo.spent_utxos).unwrap_or(0);
                total_created_utxos_size +=
                    bincode::serialized_size(&undo.created_utxos).unwrap_or(0);

                if first_samples.len() < 5 {
                    first_samples.push((height, value.to_vec()));
                }
                last_sample = Some((height, value.to_vec()));
            }
            Err(e) => {
                eprintln!("  h={height}: deserialization failed: {e}");
            }
        }
    }

    if count == 0 {
        println!("No cf_undo entries found.");
        return;
    }

    println!("Total entries:           {count}");
    println!("Height range:            {min_height}..{max_height}");
    println!(
        "Total raw size:          {} ({:.1} MB)",
        total_raw,
        total_raw as f64 / 1_048_576.0
    );
    println!(
        "Avg raw per entry:       {} ({:.1} KB)",
        total_raw / count,
        total_raw as f64 / count as f64 / 1024.0
    );
    println!();
    println!("--- Component Totals (avg per entry) ---");
    println!(
        "producer_snapshot:       {:.1} KB ({:.1}%)",
        total_producer_snap as f64 / count as f64 / 1024.0,
        total_producer_snap as f64 / total_raw as f64 * 100.0
    );
    println!(
        "epoch_state_snapshot:    {:.1} KB ({:.1}%)",
        total_epoch_snap as f64 / count as f64 / 1024.0,
        total_epoch_snap as f64 / total_raw as f64 * 100.0
    );
    println!(
        "spent_utxos:             {:.1} KB ({:.1}%)",
        total_spent_utxos_size as f64 / count as f64 / 1024.0,
        total_spent_utxos_size as f64 / total_raw as f64 * 100.0
    );
    println!(
        "created_utxos:           {:.1} KB ({:.1}%)",
        total_created_utxos_size as f64 / count as f64 / 1024.0,
        total_created_utxos_size as f64 / total_raw as f64 * 100.0
    );

    // Detailed breakdown of samples
    println!();
    println!("=== Detailed Sample Breakdown ===");

    // Add the last entry if it wasn't already in first_samples
    let mut samples = first_samples;
    if let Some((h, v)) = last_sample {
        if samples.iter().all(|(sh, _)| *sh != h) {
            samples.push((h, v));
        }
    }

    for (height, raw) in &samples {
        let undo: UndoData = bincode::deserialize(raw).unwrap();
        println!();
        println!(
            "--- Height {height} (raw: {} bytes, {:.1} KB) ---",
            raw.len(),
            raw.len() as f64 / 1024.0
        );
        println!(
            "  producer_snapshot:      {} bytes ({:.1} KB)",
            undo.producer_snapshot.len(),
            undo.producer_snapshot.len() as f64 / 1024.0
        );
        println!(
            "  epoch_state_snapshot:   {} bytes ({:.1} KB)",
            undo.epoch_state_snapshot
                .as_ref()
                .map(|s| s.len())
                .unwrap_or(0),
            undo.epoch_state_snapshot
                .as_ref()
                .map(|s| s.len())
                .unwrap_or(0) as f64
                / 1024.0
        );
        println!(
            "  spent_utxos:           {} entries, {} bytes",
            undo.spent_utxos.len(),
            bincode::serialized_size(&undo.spent_utxos).unwrap_or(0)
        );
        println!(
            "  created_utxos:         {} entries, {} bytes",
            undo.created_utxos.len(),
            bincode::serialized_size(&undo.created_utxos).unwrap_or(0)
        );

        decompose_producer_set(&undo.producer_snapshot);

        if let Some(ref es_bytes) = undo.epoch_state_snapshot {
            decompose_epoch_state(es_bytes);
        }
    }
}

fn pk_short(pk: &crypto::PublicKey) -> String {
    let b = pk.as_bytes();
    format!("{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3])
}

fn decompose_producer_set(bytes: &[u8]) {
    match bincode::deserialize::<storage::ProducerSet>(bytes) {
        Ok(ps) => {
            let producers = ps.all_producers();
            println!("  [ProducerSet] {} producers", producers.len());
            println!(
                "    total serialized: {} bytes ({:.1} KB)",
                bytes.len(),
                bytes.len() as f64 / 1024.0
            );

            // Measure individual producer serialized sizes
            #[allow(deprecated)]
            let mut per_producer_sizes: Vec<(String, usize, u32, usize, usize, usize)> = producers
                .iter()
                .map(|info| {
                    let size = bincode::serialized_size(info).unwrap_or(0) as usize;
                    (
                        pk_short(&info.public_key),
                        size,
                        info.bond_count,
                        info.bond_entries.len(),
                        info.additional_bonds.len(),
                        info.bls_pubkey.len(),
                    )
                })
                .collect();
            per_producer_sizes.sort_by_key(|p| std::cmp::Reverse(p.1));

            if !per_producer_sizes.is_empty() {
                let min = per_producer_sizes.last().unwrap().1;
                let max = per_producer_sizes.first().unwrap().1;
                let avg = per_producer_sizes.iter().map(|p| p.1).sum::<usize>()
                    / per_producer_sizes.len();
                println!("    per-producer: min={min}, avg={avg}, max={max} bytes");

                for (pk, size, bc, bonds, extra_bonds, bls) in per_producer_sizes.iter().take(5) {
                    println!(
                        "      {pk}.. = {size} bytes (bond_count={bc}, bond_entries={bonds}, extra_bonds={extra_bonds}, bls={bls})"
                    );
                }
                // Show smallest too
                if per_producer_sizes.len() > 5 {
                    let (pk, size, bc, bonds, extra_bonds, bls) =
                        per_producer_sizes.last().unwrap();
                    println!(
                        "      smallest: {pk}.. = {size} bytes (bond_count={bc}, bond_entries={bonds}, extra_bonds={extra_bonds}, bls={bls})"
                    );
                }
            }

            println!("    pending_updates: {} entries", ps.pending_update_count());
            println!("    exit_history: {} entries", ps.exit_history_size());
        }
        Err(e) => {
            eprintln!("  [ProducerSet] deserialization failed: {e}");
        }
    }
}

fn decompose_epoch_state(bytes: &[u8]) {
    match doli_core::EpochState::deserialize(bytes) {
        Ok(es) => {
            println!("  [EpochState] epoch={}", es.epoch);
            println!(
                "    total serialized: {} bytes ({:.1} KB)",
                bytes.len(),
                bytes.len() as f64 / 1024.0
            );

            let bond_snap_size = bincode::serialized_size(&es.bond_snapshot).unwrap_or(0);
            let prod_list_size = bincode::serialized_size(&es.producer_list).unwrap_or(0);
            let active_list_size = bincode::serialized_size(&es.active_list).unwrap_or(0);
            let attested_sets_size = bincode::serialized_size(&es.attested_sets).unwrap_or(0);
            let accum_size = bincode::serialized_size(&es.attestation_accum).unwrap_or(0);
            let blocks_prod_size = bincode::serialized_size(&es.blocks_produced).unwrap_or(0);

            println!(
                "    bond_snapshot:      {} bytes ({} entries)",
                bond_snap_size,
                es.bond_snapshot.len()
            );
            println!(
                "    producer_list:      {} bytes ({} entries)",
                prod_list_size,
                es.producer_list.len()
            );
            println!(
                "    active_list:        {} bytes ({} entries)",
                active_list_size,
                es.active_list.len()
            );
            println!(
                "    attested_sets:      {} bytes ([{}, {}, {}] entries)",
                attested_sets_size,
                es.attested_sets[0].len(),
                es.attested_sets[1].len(),
                es.attested_sets[2].len()
            );
            println!(
                "    attestation_accum:  {} bytes ({:.1} KB)",
                accum_size,
                accum_size as f64 / 1024.0
            );

            for (i, accum) in es.attestation_accum.iter().enumerate() {
                let accum_i_size = bincode::serialized_size(accum).unwrap_or(0);
                let total_minutes: usize = accum.values().map(|s| s.len()).sum();
                let max_minutes = accum.values().map(|s| s.len()).max().unwrap_or(0);
                let min_minutes = accum.values().map(|s| s.len()).min().unwrap_or(0);
                println!(
                    "      accum[{i}]: {} producers, {} total minutes (min={min_minutes}, max={max_minutes}), {} bytes ({:.1} KB)",
                    accum.len(),
                    total_minutes,
                    accum_i_size,
                    accum_i_size as f64 / 1024.0
                );

                // Show minute value range to check if values are absolute slot-based
                if let Some(sample) = accum.values().next() {
                    if !sample.is_empty() {
                        let min_val = sample.iter().min().copied().unwrap_or(0);
                        let max_val = sample.iter().max().copied().unwrap_or(0);
                        println!(
                            "        minute value range: {min_val}..{max_val} (span={})",
                            max_val.saturating_sub(min_val)
                        );
                    }
                }
            }

            println!(
                "    blocks_produced:    {} bytes ({} entries)",
                blocks_prod_size,
                es.blocks_produced.len()
            );

            let fields_sum = 8
                + bond_snap_size
                + prod_list_size
                + active_list_size
                + attested_sets_size
                + accum_size
                + blocks_prod_size;
            let overhead = (bytes.len() as i64) - (fields_sum as i64);
            println!(
                "    field sum:          {} bytes vs total {} bytes (overhead: {} bytes)",
                fields_sum,
                bytes.len(),
                overhead
            );
        }
        Err(e) => {
            eprintln!("  [EpochState] deserialization failed: {e}");
        }
    }
}
