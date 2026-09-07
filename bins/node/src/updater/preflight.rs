//! Startup diagnostic for the install target (INC-I-215 REQ-215-014).
//!
//! Emitted once per process start, never on a timer.

use std::path::Path;

use tracing::{debug, info, warn, Level};

/// Whether any `*.path` unit in `units_dir` watches `ready_marker`.
///
/// Matches on unit CONTENT, not on unit name: custom `--name` services and multi-node
/// hosts install helper units under arbitrary names. Any I/O error answers `false` —
/// a host that cannot be shown to have a helper does not have one.
pub fn helper_unit_watches(units_dir: &Path, ready_marker: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(units_dir) else {
        return false;
    };
    let marker = ready_marker.to_string_lossy().into_owned();
    entries.flatten().any(|entry| {
        let unit = entry.path();
        unit.extension().and_then(|e| e.to_str()) == Some("path")
            && std::fs::read_to_string(&unit).is_ok_and(|body| body.contains(&marker))
    })
}

/// The startup verdict for an install target, or `None` when the operator owes nothing.
pub fn preflight_verdict(
    target: &Path,
    target_writable: bool,
    helper_present: bool,
) -> Option<(Level, String)> {
    if target_writable {
        return None;
    }
    if helper_present {
        return Some((
            Level::INFO,
            format!(
                "Install target {} is not writable by this process: the node will stage each \
                 approved update and the helper unit will install it.",
                target.display()
            ),
        ));
    }
    Some((
        Level::WARN,
        format!(
            "UPDATE_TARGET_NOT_WRITABLE: auto-update cannot install {} because that directory \
             is not writable by this process, and no helper unit watches the staging handoff. \
             This node will never upgrade itself. Fix: run `sudo doli service install`.",
            target.display()
        ),
    ))
}

/// Log the install-target verdict for this node, once (REQ-215-014).
pub fn report_install_target(data_dir: &Path) {
    let Ok(target) = updater::current_binary_path() else {
        debug!("Install-target preflight skipped: the running binary path is unresolvable");
        return;
    };
    let marker = updater::staging_dir(data_dir).join(updater::READY_MARKER);
    let helper = helper_unit_watches(Path::new("/etc/systemd/system"), &marker);
    if let Some((level, msg)) =
        preflight_verdict(&target, updater::target_dir_is_writable(&target), helper)
    {
        if level == Level::WARN {
            warn!("{msg}");
        } else {
            info!("{msg}");
        }
    }
}
