# DOLI Fuzzing

This directory contains fuzzing harnesses for DOLI components using [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html) and libFuzzer.

## Prerequisites

Install cargo-fuzz:

```bash
cargo install cargo-fuzz
```

Fuzzing requires nightly Rust:

```bash
rustup install nightly
```

## Available Fuzz Targets

| Target | Description |
|--------|-------------|
| `fuzz_block_deserialize` | Block deserialization from arbitrary bytes |
| `fuzz_tx_deserialize` | Transaction deserialization from arbitrary bytes |
| `fuzz_vdf_verify` | VDF verification with arbitrary proofs |
| `fuzz_merkle` | Merkle tree computation with arbitrary transactions |
| `fuzz_signature` | Signature verification with arbitrary keys/signatures |
| `fuzz_hash` | Hash function with arbitrary inputs |

## Running Fuzzers

Run a specific fuzzer:

```bash
cd fuzz
cargo +nightly fuzz run fuzz_block_deserialize
```

Run with a time limit (e.g., 60 seconds):

```bash
cargo +nightly fuzz run fuzz_tx_deserialize -- -max_total_time=60
```

Run with multiple jobs (parallel fuzzing):

```bash
cargo +nightly fuzz run fuzz_hash -- -jobs=4 -workers=4
```

## Viewing Crashes

If a crash is found, it will be saved in `fuzz/artifacts/<target_name>/`.

To reproduce a crash:

```bash
cargo +nightly fuzz run fuzz_block_deserialize artifacts/fuzz_block_deserialize/crash-xxxxx
```

## Code Coverage

Generate coverage report:

```bash
cargo +nightly fuzz coverage fuzz_tx_deserialize
```

View the coverage:

```bash
cargo +nightly cov -- show target/coverage/fuzz_tx_deserialize \
    --format=html \
    -instr-profile=fuzz/coverage/fuzz_tx_deserialize/coverage.profdata \
    > coverage.html
```

## Corpus Management

The corpus (set of interesting inputs) is stored in `fuzz/corpus/<target_name>/`.

Minimize the corpus:

```bash
cargo +nightly fuzz cmin fuzz_block_deserialize
```

## Tips for Effective Fuzzing

1. **Run for extended periods**: Fuzzers are more effective with longer run times (hours/days)

2. **Use multiple cores**: Run parallel instances with `-jobs` and `-workers`

3. **Check coverage**: Use coverage reports to identify untested code paths

4. **Seed with valid inputs**: Add valid examples to the corpus for better coverage

5. **Monitor memory usage**: Some targets may consume significant memory

## Adding New Fuzz Targets

1. Create a new file in `fuzz_targets/`
2. Add the `[[bin]]` entry to `fuzz/Cargo.toml`
3. Use `libfuzzer_sys::fuzz_target!` macro
4. Consider using `arbitrary` crate for structured fuzzing

Example:

```rust
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Your fuzzing logic here
    // Should not panic on any input
});
```

## Security Reporting

If you find a security vulnerability through fuzzing, please report it according to our [security policy](../SECURITY.md).
