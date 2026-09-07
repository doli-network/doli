//! INC-I-215 M4 outcome-probe fixture: render the two helper units to a scratch dir.
//!
//! argv: `<out_dir> <service_name> <network> <data_dir> <cli_path>`. It uses only the pure
//! content functions, so it never touches /etc and never shells out to systemctl.
//!
//! OUTPUT CONTRACT: N/A — fixture file.

use std::path::{Path, PathBuf};

use doli_cli::cmd_service_helper_units as units;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    assert_eq!(
        a.len(),
        5,
        "usage: <out_dir> <service_name> <network> <data_dir> <cli_path>"
    );
    let (out, svc, network) = (PathBuf::from(&a[0]), a[1].as_str(), a[2].as_str());
    let (data_dir, cli) = (PathBuf::from(&a[3]), PathBuf::from(&a[4]));

    let (path_name, service_name) = units::helper_unit_names(svc);
    write(
        &out.join(path_name),
        &units::helper_path_unit_content(svc, &data_dir),
    );
    write(
        &out.join(service_name),
        &units::helper_service_unit_content(svc, network, &data_dir, &cli),
    );
    println!("ok");
}

fn write(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
