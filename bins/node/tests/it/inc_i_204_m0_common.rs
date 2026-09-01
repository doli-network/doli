//! INC-I-204 M0 — shared Prometheus-registry scraping fixture.
//!
//! OUTPUT CONTRACT: N/A — fixture file. It declares no `#[test]`; the
//! enumerations live with the functions under test in the sibling
//! `inc_i_204_m0_*` modules. INPUT PARTITIONS: N/A — fixture file.
//!
//! `encode_registry` renders what `metrics_handler` (metrics.rs:808-818) serves:
//! the same `REGISTRY.gather()`, the same exposition lines. Every assertion in
//! this milestone reads the RENDERED text, so "registered" can never be mistaken
//! for "exported" — the INC-I-187 failure (28 of 57 `doli_*` metrics registered
//! and never written, reading 0 on healthy nodes).

#![allow(dead_code)] // each consumer uses a subset

use std::sync::Once;

use doli_node::metrics::{register_metrics, REGISTRY};

static REG: Once = Once::new();

/// Register once per test binary. `register_metrics` swallows duplicate
/// registration (`let _ = REGISTRY.register(..)`), but one call keeps the
/// rendered text stable across parallel tests.
pub fn ensure_registered() {
    REG.call_once(register_metrics);
}

/// The exposition text an operator's Prometheus would scrape from this process.
///
/// Counter, gauge and untyped are mutually exclusive on the wire and an absent
/// field decodes to its 0.0 default, so their sum is the series scalar.
/// Histogram/summary render 0 and are out of M0 scope. The `prometheus` types are
/// reached by method call only — the crate is not a dev-dependency here.
pub fn encode_registry() -> String {
    ensure_registered();
    let mut out = String::new();
    for mf in REGISTRY.gather() {
        out.push_str(&format!("# HELP {} {}\n", mf.get_name(), mf.get_help()));
        for m in mf.get_metric() {
            let scalar = m.get_counter().get_value()
                + m.get_gauge().get_value()
                + m.get_untyped().get_value();
            let labels: Vec<String> = m
                .get_label()
                .iter()
                .map(|l| format!("{}=\"{}\"", l.get_name(), l.get_value()))
                .collect();
            if labels.is_empty() {
                out.push_str(&format!("{} {}\n", mf.get_name(), scalar));
            } else {
                out.push_str(&format!(
                    "{}{{{}}} {}\n",
                    mf.get_name(),
                    labels.join(","),
                    scalar
                ));
            }
        }
    }
    out
}

/// HELP text of a family, or `None` when the family publishes nothing.
pub fn help_text(name: &str) -> Option<String> {
    ensure_registered();
    REGISTRY
        .gather()
        .iter()
        .find(|mf| mf.get_name() == name)
        .map(|mf| mf.get_help().to_string())
}

/// Value of the series identified by `name` + the exact label set, read back out
/// of the RENDERED exposition text — not out of the counter handle. A handle read
/// would pass on a metric that is registered but never collected.
pub fn exported_value(name: &str, labels: &[(&str, &str)]) -> Option<f64> {
    let text = encode_registry();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let (head, raw_value) = line.rsplit_once(' ')?;
        let (family, label_blob) = match head.split_once('{') {
            Some((f, rest)) => (f, rest.trim_end_matches('}')),
            None => (head, ""),
        };
        if family != name {
            continue;
        }
        let matches_all = labels
            .iter()
            .all(|(k, v)| label_blob.split(',').any(|p| p == format!("{k}=\"{v}\"")));
        if matches_all {
            return raw_value.parse::<f64>().ok();
        }
    }
    None
}

/// Every distinct value `label` takes across the exported series of `name`.
pub fn exported_label_values(name: &str, label: &str) -> Vec<String> {
    ensure_registered();
    let mut found: Vec<String> = REGISTRY
        .gather()
        .iter()
        .filter(|mf| mf.get_name() == name)
        .flat_map(|mf| mf.get_metric().iter())
        .flat_map(|m| m.get_label().iter())
        .filter(|l| l.get_name() == label)
        .map(|l| l.get_value().to_string())
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The acceptance edge for every M0 metric: the series must be PRESENT in the
/// exposition text AND carry a non-default value, after the real writing path ran.
pub fn assert_exported_nonzero(name: &str, labels: &[(&str, &str)], ctx: &str) {
    match exported_value(name, labels) {
        None => panic!(
            "{ctx}: `{name}{labels:?}` is ABSENT from the exported registry. \
             Driving the real path must publish the series, not merely register the family.\n\
             --- exported ---\n{}",
            encode_registry()
        ),
        // `assert!` rather than a `Some(v) if v == 0.0` arm: clippy rejects the
        // guard, and a float literal pattern is future-incompatible.
        Some(v) => assert!(
            v != 0.0,
            "{ctx}: `{name}{labels:?}` is exported but still 0 after the real path ran — \
             registered-and-never-written, the INC-I-187 shape."
        ),
    }
}
