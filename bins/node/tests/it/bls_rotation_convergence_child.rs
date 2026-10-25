//! Child-process dispatch for `bls_rotation_convergence`: each test re-runs alone
//! with the rotation gate env set.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: rewards rollback block_handling state_update post_commit

use super::bls_rotation_convergence::ROT_AH;
use crate::inc_i_256_m2_common::{child, in_child};

/// In the parent: runs `bls_rotation_convergence::{test}` in a child process with
/// `DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT = ROT_AH`, then returns true. In the child: false.
pub fn ran_in_child(test: &str) -> bool {
    if in_child() {
        return false;
    }
    let env = [(
        "DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT",
        ROT_AH.to_string(),
    )];
    child(&format!("bls_rotation_convergence::{test}"), &env);
    true
}
