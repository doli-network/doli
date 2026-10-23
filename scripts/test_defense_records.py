#!/usr/bin/env python3
"""pytest entry points for the M2 bash checks: whitepaper defense text and gauntlet seed honesty.

Run: python3 -m pytest -q -p no:cacheprovider scripts/test_defense_records.py
"""
# REQ-LDP-046 — Decision: a FAIL means scripts/test_whitepaper_defense_text.sh exits non-zero on the committed whitepapers.
# REQ-LDP-050 — Decision: a FAIL means scripts/test_gauntlet_seed_optin.sh exits non-zero on the committed seed and gauntlet.sh.
# OUTPUT CONTRACT: each test asserts the bash suite's exit code is 0 and prints its PASS/FAIL lines.
# INPUT PARTITIONS: suite exit 0 (pass) · exit 1 (a check failed) · timeout (fails).
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def _run(script):
    p = subprocess.run(["bash", os.path.join(ROOT, "scripts", script)],
                       capture_output=True, text=True, timeout=120)
    print(p.stdout)
    print(p.stderr)
    return p.returncode


def test_whitepaper_defense_text():
    assert _run("test_whitepaper_defense_text.sh") == 0, "test_whitepaper_defense_text.sh failed"


def test_gauntlet_seed_optin():
    assert _run("test_gauntlet_seed_optin.sh") == 0, "test_gauntlet_seed_optin.sh failed"
