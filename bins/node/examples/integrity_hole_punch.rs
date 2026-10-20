//! GS-025 (INC-I-253) helper: delete one block from a STOPPED scratch block store.
//!
//! Usage: `integrity_hole_punch <BLOCKS_DB_DIR> <HEIGHT>`.
//! Prints `PUNCHED height=<h> hash=<hex>` (exit 0), `ABSENT height=<h>` (exit 1)
//! or `REFUSE <reason>` (exit 1). Refused before any open: `$HOME/testnet` and every
//! colon-separated root in `GS025_PROTECTED_ROOTS`, a dir without `CURRENT`, a held `LOCK`.

use std::fmt::Display;
use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::exit;

use fs2::FileExt;
use storage::BlockStore;

const USAGE: &str = "usage: integrity_hole_punch <BLOCKS_DB_DIR> <HEIGHT>";

fn refuse(why: impl Display) -> ! {
    println!("REFUSE {why}");
    exit(1);
}

fn protected_roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
    else {
        refuse("cannot resolve $HOME, so $HOME/testnet cannot be protected");
    };
    let mut roots = vec![home.join("testnet")];
    if let Some(extra) = std::env::var_os("GS025_PROTECTED_ROOTS") {
        roots.extend(std::env::split_paths(&extra).filter(|p| !p.as_os_str().is_empty()));
    }
    roots
}

fn lock_holder(dir: &Path) -> Option<String> {
    let path = dir.join("LOCK");
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return None,
        Err(e) => return Some(format!("cannot open lock file {}: {e}", path.display())),
    };
    match file.try_lock_exclusive() {
        Ok(()) => None,
        Err(e) => Some(format!(
            "lock {} is held by another process: {e}",
            path.display()
        )),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, height] = args.as_slice() else {
        eprintln!("{USAGE}");
        exit(2);
    };
    let Ok(height) = height.parse::<u64>() else {
        eprintln!("{USAGE} (HEIGHT must be an unsigned integer, got {height:?})");
        exit(2);
    };

    let dir = match std::fs::canonicalize(dir) {
        Ok(d) => d,
        Err(e) => refuse(format_args!("{dir}: cannot resolve the path: {e}")),
    };
    for root in protected_roots() {
        let canon = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        if dir.starts_with(&canon) || dir.starts_with(&root) {
            refuse(format_args!(
                "{} is under the protected root {}",
                dir.display(),
                canon.display()
            ));
        }
    }
    if !dir.join("CURRENT").is_file() {
        refuse(format_args!(
            "{} has no CURRENT file: not a RocksDB store",
            dir.display()
        ));
    }
    if let Some(why) = lock_holder(&dir) {
        refuse(why);
    }

    let store = match BlockStore::open(&dir) {
        Ok(s) => s,
        Err(e) if e.to_string().to_lowercase().contains("lock") => {
            refuse(format_args!("store lock is held: {e}"))
        }
        Err(e) => refuse(format_args!("cannot open {}: {e}", dir.display())),
    };
    let code = match store.get_hash_by_height(height) {
        Err(e) => {
            println!("ERROR height={height}: {e}");
            1
        }
        Ok(hash) => match store.delete_block_by_height(height) {
            Ok(true) => {
                let hex = hash.map_or_else(|| "none".to_string(), |h| h.to_hex());
                println!("PUNCHED height={height} hash={hex}");
                0
            }
            Ok(false) => {
                println!("ABSENT height={height}");
                1
            }
            Err(e) => {
                println!("ERROR height={height}: {e}");
                1
            }
        },
    };
    drop(store);
    exit(code);
}
