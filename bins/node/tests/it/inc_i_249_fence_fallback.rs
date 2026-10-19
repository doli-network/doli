// covers: archive_thread.rs fence.rs
//! INC-I-249 M5: a foreign archive that can be neither moved nor emptied
//! never gets an archive thread.
//!
//! OUTPUT CONTRACT: start_archive_thread -> Ok | Err; node.archive_tx;
//!   archive dir entries; manifest.json genesis_hash.
//! INPUT PARTITIONS: parent unwritable + locked subdir, store holds 1..=tip.

use std::collections::BTreeSet;
use std::fs::{File, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crypto::KeyPair;
use doli_core::Block;
use doli_node::node::archive_thread::start_archive_thread;
use doli_node::node::Node;
use tempfile::TempDir;

use super::inc_i_249_archive_fixture as fx;

const TIP: u64 = 3;

struct Unlock(Vec<PathBuf>);

impl Drop for Unlock {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::set_permissions(p, Permissions::from_mode(0o755));
        }
    }
}

fn lock(paths: &[&Path]) -> Unlock {
    for p in paths {
        std::fs::set_permissions(p, Permissions::from_mode(0o555)).unwrap();
    }
    Unlock(paths.iter().map(|p| p.to_path_buf()).collect())
}

fn entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

// REQ-ARCH-005 REQ-ARCH-002 — Decision: a failure means an archive thread writes into a foreign archive the fence could not remove (AUDIT-P2-525).
#[tokio::test(flavor = "current_thread")]
async fn start_refuses_archive_when_fence_cannot_move_or_empty_it() {
    let tmp = TempDir::new().unwrap();
    let mut node = Node::new_for_test(tmp.path().join("node"), vec![KeyPair::generate()])
        .await
        .expect("Node::new_for_test");
    let genesis = node.params.genesis_hash;
    for h in 1..=TIP {
        node.block_store()
            .put_block_canonical(&fx::block_at(genesis, h), h)
            .unwrap();
    }
    let foreign = crypto::hash::hash(b"inc-i-249-m5-foreign-genesis");
    assert_ne!(foreign, genesis);
    let parent = tmp.path().join("parent");
    let archive = parent.join("archive");
    fx::build_legacy_archive(&archive, TIP, foreign);
    let locked = archive.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("f"), b"x").unwrap();
    let before = entries(&archive);
    let _unlock = lock(&[&locked, &parent]);
    let probe = locked.join(".perm-probe");
    if File::create(&probe).is_ok() {
        let _ = std::fs::remove_file(&probe);
        println!("SKIP: a 0555 dir is writable here (root); the fence cannot be made to fail");
        return;
    }

    let res = start_archive_thread(&mut node, archive.clone(), TIP);
    let started = res.is_ok();
    let tx_set = node.archive_tx.is_some();
    tokio::time::sleep(Duration::from_millis(300)).await;
    if let Ok(at) = res {
        node.archive_tx = None;
        fx::wait_until(Duration::from_secs(30), || at.thread.is_finished()).await;
    }

    assert!(
        !started,
        "start_archive_thread returned Ok although the fence could neither move nor empty the foreign archive"
    );
    assert!(!tx_set, "archive_tx set for a refused archive");
    let added: Vec<String> = entries(&archive).difference(&before).cloned().collect();
    assert!(
        added.is_empty(),
        "new entries in the foreign archive: {added:?}"
    );
    let ours = (1..=TIP)
        .filter(|h| {
            std::fs::read(fx::block_path(&archive, *h))
                .ok()
                .and_then(|d| bincode::deserialize::<Block>(&d).ok())
                .is_some_and(|b| b.header.genesis_hash == genesis)
        })
        .count();
    assert_eq!(
        ours, 0,
        "blocks of our genesis written into the foreign archive"
    );
    let manifest_genesis = fx::read_manifest(&archive)
        .and_then(|m| m.get("genesis_hash")?.as_str().map(str::to_string));
    assert!(
        manifest_genesis.is_none() || manifest_genesis == Some(foreign.to_string()),
        "foreign archive manifest now names {manifest_genesis:?}, not the foreign genesis"
    );
}
