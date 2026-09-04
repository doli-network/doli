#!/usr/bin/env bash
# INC-I-178 M0 outcome probe (run 544).
# Metric: number of published attestation baseline vectors — reference bytes emitted by
# the CURRENT production encoder/hasher, consumable outside the pipeline (a node operator
# or any future binary can diff a build against them to prove pre-AH byte identity,
# REQ-BLS-005 AC-1). Counts rows in the store; not a test count.
STORE=/Users/isudoajl/ownCloud/Projects/doli-network/doli/crates/core/tests/fixtures/attestation_baseline_vectors.json
if [ ! -f "$STORE" ]; then echo "BASELINE_VECTORS=0"; exit 0; fi
python3 - "$STORE" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))
print("BASELINE_VECTORS=%d" % len(d.get("vectors",[])))
PY
