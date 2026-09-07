//! INC-I-215 M2 outcome probe: how many actionable startup diagnostics does the node
//! emit when its install target is not writable?
//!
//! Usage: `inc_i_215_preflight_probe <target-binary-path> <data-dir> <units-dir>`.
//! Prints the shipped preflight verdict, or nothing when none is owed.

use std::path::Path;

use doli_node::updater::preflight::{helper_unit_watches, preflight_verdict};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [target, data_dir, units_dir] = args.as_slice() else {
        eprintln!("usage: inc_i_215_preflight_probe <target> <data-dir> <units-dir>");
        std::process::exit(2);
    };

    let target = Path::new(target);
    let marker = updater::staging_dir(Path::new(data_dir)).join(updater::READY_MARKER);
    let helper = helper_unit_watches(Path::new(units_dir), &marker);

    if let Some((_, msg)) =
        preflight_verdict(target, updater::target_dir_is_writable(target), helper)
    {
        println!("{msg}");
    }
}
