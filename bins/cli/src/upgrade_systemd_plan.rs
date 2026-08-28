//! Pure systemd command plan consumed by the `doli upgrade` restart path.

/// One `sudo`-prefixed invocation in a systemd restart plan.
#[derive(Debug)]
pub struct SystemdStep {
    /// Argv passed after `sudo`; always starts with `"systemctl"`.
    pub args: Vec<String>,
    /// `false` means best-effort: the caller runs the step and ignores its exit status.
    pub required: bool,
}

/// Plan for `unit`: best-effort `reset-failed`, then required `restart`.
/// A unit in start-limit lock refuses a plain `restart`; only `reset-failed` clears it.
pub fn systemd_restart_plan(unit: &str) -> Vec<SystemdStep> {
    vec![
        SystemdStep {
            args: vec![
                "systemctl".to_string(),
                "reset-failed".to_string(),
                unit.to_string(),
            ],
            required: false,
        },
        SystemdStep {
            args: vec![
                "systemctl".to_string(),
                "restart".to_string(),
                unit.to_string(),
            ],
            required: true,
        },
    ]
}
