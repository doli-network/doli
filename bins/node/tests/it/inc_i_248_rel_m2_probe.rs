//! INC-I-248 REL-M2 outcome-metric probe (not an assertion test).
//!
//! Returns at once unless `INC_I_248_REL_M2_PROBE=1`. Prints one line and asserts nothing.
//!
//! OUTPUT CONTRACT: N/A — probe, prints one line, no assertion.
//! INPUT PARTITIONS: N/A — one fixed input (stale pre-fix snap data dir: producer seed at
//! h=1, store = block 1 + S+1..=TIP, 1 sync peer, startup tick at +11 s, 3 honest
//! canonical answers, window-end tick at +30 s).
// covers: bins/node/src/node/maintainer_sanitize.rs, bins/node/src/run.rs (start-time sanitize)

use std::time::Instant;

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m1_fixture as fx;
use crate::inc_i_248_rel_m2_fixture as m2;

fn short(digest: [u8; 32]) -> String {
    hex::encode(digest)[..16].to_string()
}

#[tokio::test]
async fn inc_i_248_rel_m2_probe_stale_snap_datadir() {
    if std::env::var("INC_I_248_REL_M2_PROBE").ok().as_deref() != Some("1") {
        println!("INC-I-248-REL-M2-PROBE=skipped (set INC_I_248_REL_M2_PROBE=1)");
        return;
    }
    let (mut node, _temp) = m2::stale_snap_node().await;
    node.sanitize_retired_maintainer_set().await;

    fx::add_sync_peer(&node, m2::TIP).await;
    let t0 = Instant::now();
    node.maintainer_quorum_tick_at(t0 + fx::STARTUP).await;
    let canonical = fx::canonical(h::last_rotation_height());
    let good = fx::frame(&node, &canonical, m2::TIP);
    h::deliver(&mut node, &good, 3).await;
    node.maintainer_quorum_tick_at(t0 + fx::WINDOW_END).await;

    let st = h::state_of(&node).await;
    let digest = if st.set.members.is_empty() {
        "empty".to_string()
    } else {
        short(h::node_digest(&node, &st.set))
    };
    println!(
        "INC-I-248-REL-M2 stale-snap-datadir digest={digest} last_change_block={} canonical={}",
        st.set.last_updated,
        short(h::node_digest(&node, &canonical))
    );
}
