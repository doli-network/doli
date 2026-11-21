//! Admin RPC token guidance printed after `doli service install`.

use std::path::Path;

/// Print where the admin token lives and how to reach the RPC remotely.
pub fn print(token_file: &Path) {
    println!(
        "  Admin RPC: methods like guardian halt/resume need the token in {} (owner-only, created by the node on first start).",
        token_file.display()
    );
    println!("  The doli CLI reads it automatically (or set DOLI_RPC_ADMIN_TOKEN).");
    println!("  Remote access: ssh -L 8500:127.0.0.1:8500 <host> — never bind the RPC to 0.0.0.0.");
}
