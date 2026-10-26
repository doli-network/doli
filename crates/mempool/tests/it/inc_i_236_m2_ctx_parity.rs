//! INC-I-236 M2 — INV-PROD-003: both mempool `ValidationContext` sites thread
//! the block-signature AH from NetworkParams.
//!
// covers: crates/mempool/src/pool.rs
//
// OUTPUT CONTRACT: N/A — source scan. Each construction site is one cell.
// INPUT PARTITIONS: site in {pool.rs admission, pool.rs revalidation}.

const FIELD: &str = "inc_i_236_block_signature_activation_height";

fn sites(src: &str) -> Vec<String> {
    let end = src.find("#[cfg(test)]").unwrap_or(src.len());
    let code: String = src[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let starts: Vec<usize> = code
        .match_indices("ValidationContext::new(")
        .map(|(i, _)| i)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(k, &s)| {
            let rest = &code[s..];
            let next = starts.get(k + 1).map(|&n| n - s).unwrap_or(rest.len());
            let fn_end = rest.find("\n    }\n").unwrap_or(rest.len());
            rest[..next.min(fn_end)].to_string()
        })
        .collect()
}

// REQ-AUTH-001 — Decision: a failure names the mempool ctx site that keeps AH = u64::MAX (mempool/builder/apply disagree above the AH).
#[test]
fn req_auth_001_mempool_pool_sites_thread_block_sig_ah() {
    let found = sites(include_str!("../../src/pool.rs"));
    assert_eq!(
        found.len(),
        2,
        "NON-VACUITY: pool.rs must hold 2 non-test ValidationContext sites; re-audit if this moved"
    );
    for (i, site) in found.iter().enumerate() {
        assert!(
            site.contains(FIELD),
            "REQ-AUTH-001 / INV-PROD-003: pool.rs site #{i} does not set {FIELD} from NetworkParams"
        );
    }
}

// REQ-AUTH-001 — Decision: a failure means the segments miss the builder chain (threaded INC-I-234 sibling unseen), so the pool.rs cell is vacuous.
#[test]
fn req_auth_001_mempool_scanner_sees_sibling_inc_i_234_threading() {
    for (i, site) in sites(include_str!("../../src/pool.rs")).iter().enumerate() {
        assert!(
            site.contains("inc_i_234_activation_height"),
            "pool.rs site #{i}: segment misses the chain"
        );
    }
}

// REQ-AUTH-001 — Decision: a failure means the scanner is blind, so the pool.rs cell proves nothing.
#[test]
fn req_auth_001_mempool_scanner_catches_planted_unthreaded_site() {
    let planted =
        "fn a() {\n    let c = ValidationContext::new(p, n, 0, h)\n        .with_x(1);\n    }\n";
    assert!(!sites(planted)[0].contains(FIELD));
    let fixed = planted.replace(
        ".with_x(1)",
        ".with_inc_i_236_block_signature_activation_height(p)",
    );
    assert!(sites(&fixed)[0].contains(FIELD));
}
