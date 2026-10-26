//! INC-I-236 M2 — INV-PROD-003: every node `ValidationContext` site threads the
//! block-signature AH. Source falsifier: a site left at the u64::MAX default
//! silently disables the rule on that path.
//!
// covers: bins/node/src/node/validation_checks/mod.rs bins/node/src/node/apply_block/tx_processing.rs bins/node/src/node/production/assembly.rs
//
// OUTPUT CONTRACT: N/A — source scan. Each construction site is one cell.
// INPUT PARTITIONS: site in {validation_checks x2, tx_processing, assembly}.

const FIELD: &str = "inc_i_236_block_signature_activation_height";

/// Non-test source, comment lines removed.
fn code(src: &str) -> String {
    let end = src.find("#[cfg(test)]").unwrap_or(src.len());
    src[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text of each construction site up to the end of its enclosing fn.
pub fn sites(src: &str) -> Vec<String> {
    let code = code(src);
    let marker = "ValidationContext::new(";
    let starts: Vec<usize> = code.match_indices(marker).map(|(i, _)| i).collect();
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

fn assert_threaded(file: &str, src: &str, expected_sites: usize) {
    let found = sites(src);
    assert_eq!(
        found.len(),
        expected_sites,
        "NON-VACUITY: {file} must hold {expected_sites} non-test ValidationContext sites; re-audit if this moved"
    );
    for (i, site) in found.iter().enumerate() {
        assert!(
            site.contains(FIELD),
            "REQ-AUTH-001 / INV-PROD-003: {file} site #{i} does not set {FIELD} from NetworkParams"
        );
    }
}

// REQ-AUTH-001 — Decision: a failure names the node ctx site that keeps AH = u64::MAX (builder/apply/validate disagree above the AH).
#[test]
fn req_auth_001_validation_checks_sites_thread_block_sig_ah() {
    assert_threaded(
        "validation_checks/mod.rs",
        include_str!("../../src/node/validation_checks/mod.rs"),
        2,
    );
}

// REQ-AUTH-001 — Decision: a failure means the apply-path tx ctx ignores the AH (apply/validate divergence).
#[test]
fn req_auth_001_tx_processing_site_threads_block_sig_ah() {
    assert_threaded(
        "apply_block/tx_processing.rs",
        include_str!("../../src/node/apply_block/tx_processing.rs"),
        1,
    );
}

// REQ-AUTH-001 — Decision: a failure means the builder ctx ignores the AH (builder/apply divergence).
#[test]
fn req_auth_001_assembly_site_threads_block_sig_ah() {
    assert_threaded(
        "production/assembly.rs",
        include_str!("../../src/node/production/assembly.rs"),
        1,
    );
}

// REQ-AUTH-001 — Decision: a failure means the site segments miss the builder chain (the threaded INC-I-234 sibling is not seen), so the cells above are vacuous.
#[test]
fn req_auth_001_scanner_sees_sibling_inc_i_234_threading() {
    for (file, src) in [
        (
            "validation_checks",
            include_str!("../../src/node/validation_checks/mod.rs"),
        ),
        (
            "tx_processing",
            include_str!("../../src/node/apply_block/tx_processing.rs"),
        ),
        (
            "assembly",
            include_str!("../../src/node/production/assembly.rs"),
        ),
    ] {
        for (i, site) in sites(src).iter().enumerate() {
            assert!(
                site.contains("inc_i_234_activation_height"),
                "{file} site #{i}: segment misses the chain"
            );
        }
    }
}

// REQ-AUTH-001 — Decision: a failure means the scanner is blind (a planted unthreaded site passes), so the other cells prove nothing.
#[test]
fn req_auth_001_scanner_catches_planted_unthreaded_site() {
    let planted =
        "fn a() {\n    let c = ValidationContext::new(p, n, 0, h)\n        .with_x(1);\n    }\n";
    let found = sites(planted);
    assert_eq!(found.len(), 1);
    assert!(
        !found[0].contains(FIELD),
        "scanner must report the planted site as unthreaded"
    );
    let fixed = planted.replace(
        ".with_x(1)",
        ".with_inc_i_236_block_signature_activation_height(p)",
    );
    assert!(
        sites(&fixed)[0].contains(FIELD),
        "scanner must accept a threaded site"
    );
}
