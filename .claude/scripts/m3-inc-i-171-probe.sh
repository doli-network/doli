#!/usr/bin/env bash
# INC-I-171 M3 outcome probe (run 547).
#
# Metric: duplicated withdrawal-input scan sites in SHIPPED (non-test) Rust.
# Counts every non-test source line naming `bond_input_split` -- the private
# per-crate copy of the "scan tx.inputs, count Bond UTXOs" loop. Each copy is
# a place where mempool admission, block building and block validation can
# drift into three different verdicts for one transaction (INV-VEST-008).
#
# Source-level by design: M3 is behaviour-preserving, so no runtime number
# moves. The behavioural evidence is the oracle-parity test, which pins the
# resolver's counts to the pre-M3 inline scan.
#
# Externally observable: any integrator counts it on a plain checkout with no
# build, no test run and no node. Pipe-free at the call site so the value can
# be quoted in a commit message.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# `|| true`: grep exits 1 on zero matches, which is this metric's SUCCESS state.
{ grep -rn 'bond_input_split' crates bins --include='*.rs' --exclude-dir=tests || true; } | wc -l | tr -d ' '
