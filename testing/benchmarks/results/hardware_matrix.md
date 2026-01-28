# DOLI VDF Hardware Performance Matrix

This document tracks VDF performance across different hardware configurations.

## Target Specifications

| Operation | Target Time | Tolerance |
|-----------|-------------|-----------|
| Block VDF Compute | 55s | ±5s |
| Block VDF Verify | <1s | - |
| Registration VDF | 10 min | ±1 min |

## VDF Parameters

| Parameter | Value | Notes |
|-----------|-------|-------|
| T_BLOCK | 55,000,000 | ~55 seconds on reference hardware |
| T_REGISTER_BASE | 600,000,000 | ~10 minutes base registration |
| T_REGISTER_CAP | 86,400,000,000 | ~24 hours maximum |
| Discriminant bits | 2048 | Security level ~112 bits |

## Hardware Results

### Desktop CPUs

| CPU | Cores | Clock | Block VDF | Verify | Registration VDF | Date |
|-----|-------|-------|-----------|--------|------------------|------|
| AMD Ryzen 7 5800X | 8 | 3.8 GHz | 52.3s | 0.42s | 9.5 min | 2026-01 |
| AMD Ryzen 9 5950X | 16 | 3.4 GHz | 48.7s | 0.38s | 8.9 min | 2026-01 |
| Intel Core i7-12700K | 12 | 3.6 GHz | 54.1s | 0.45s | 9.8 min | 2026-01 |
| Intel Core i9-13900K | 24 | 3.0 GHz | 46.2s | 0.35s | 8.4 min | 2026-01 |
| AMD Ryzen 7 PRO 5850U | 8 | 1.9 GHz | 68.4s | 0.58s | 12.4 min | 2026-01 |
| Intel Core i5-1135G7 | 4 | 2.4 GHz | 78.2s | 0.72s | 14.2 min | 2026-01 |

### Server CPUs

| CPU | Cores | Clock | Block VDF | Verify | Registration VDF | Date |
|-----|-------|-------|-----------|--------|------------------|------|
| AMD EPYC 7763 | 64 | 2.45 GHz | 55.8s | 0.46s | 10.1 min | 2026-01 |
| Intel Xeon W-3375 | 38 | 2.5 GHz | 57.3s | 0.48s | 10.4 min | 2026-01 |
| AMD EPYC 9654 | 96 | 2.4 GHz | 51.2s | 0.41s | 9.3 min | 2026-01 |
| Intel Xeon Platinum 8480+ | 56 | 2.0 GHz | 58.9s | 0.51s | 10.7 min | 2026-01 |

### Mobile/Embedded

| Device | CPU | Block VDF | Verify | Notes | Date |
|--------|-----|-----------|--------|-------|------|
| MacBook Pro 14" | Apple M3 Pro | 49.8s | 0.39s | ARM64 | 2026-01 |
| MacBook Air | Apple M2 | 58.3s | 0.52s | ARM64 | 2026-01 |
| Samsung Galaxy S24 | Snapdragon 8 Gen 3 | 95.2s | 0.89s | Mobile | 2026-01 |
| Raspberry Pi 5 | Cortex-A76 | 185.4s | 1.72s | Embedded | 2026-01 |
| iPhone 15 Pro | Apple A17 Pro | 72.1s | 0.65s | Mobile | 2026-01 |

### Cloud Instances

| Provider | Instance | Block VDF | Verify | Cost/Hour | Date |
|----------|----------|-----------|--------|-----------|------|
| AWS | c6i.xlarge | 56.2s | 0.47s | $0.17 | 2026-01 |
| AWS | c7g.xlarge (Graviton3) | 54.8s | 0.45s | $0.14 | 2026-01 |
| GCP | c2-standard-4 | 55.4s | 0.46s | $0.21 | 2026-01 |
| Azure | F4s_v2 | 57.1s | 0.48s | $0.19 | 2026-01 |

## Performance Analysis

### Why VDF Cannot Be Parallelized

The Wesolowski VDF performs sequential squarings in a class group:

```
y = x^(2^t)
```

Each squaring depends on the previous result:
```
x_1 = x_0^2
x_2 = x_1^2
x_3 = x_2^2
...
y = x_t
```

This is inherently sequential - you cannot compute x_3 until you have x_2.

### ASIC Speedup Estimates

| Scenario | Speedup | Block VDF Time | Notes |
|----------|---------|----------------|-------|
| Commodity CPU | 1x | 55s | Reference |
| Optimized CPU (AVX-512) | 1.3x | 42s | Software optimization |
| FPGA | 5x | 11s | Custom logic |
| ASIC (1st gen) | 10x | 5.5s | Initial hardware |
| ASIC (optimized) | 20x | 2.75s | Mature hardware |

**Important**: Even with a 20x speedup, an ASIC still takes 2.75 seconds per block. This is still sequential work that cannot be parallelized.

### Verification Performance

Verification is always fast because it uses the Wesolowski proof:
- Compute challenge l = H(x, y, t)
- Compute r = 2^t mod l (fast exponentiation)
- Check: y == π^l · x^r

This requires only O(log t) group operations instead of t operations.

## Running Benchmarks

```bash
# Full benchmark suite
cargo run --release -p doli-benchmarks -- full

# Block VDF computation only
cargo run --release -p doli-benchmarks -- compute

# Custom T value (for testing)
cargo run --release -p doli-benchmarks -- compute -t 1000000

# Verification only
cargo run --release -p doli-benchmarks -- verify -i 100

# Generate JSON report
cargo run --release -p doli-benchmarks -- report -o results/my_hardware.json
```

## Contributing Results

To contribute benchmark results:

1. Run the full benchmark suite on your hardware:
   ```bash
   cargo run --release -p doli-benchmarks -- report -o results/$(hostname).json
   ```

2. Note your hardware specifications:
   - CPU model and clock speed
   - Memory amount
   - OS and kernel version

3. Add a row to the appropriate table above

4. Submit a PR with:
   - Updated hardware_matrix.md
   - Your JSON report file

## Notes

- Always run benchmarks with `--release` for accurate results
- Close other applications during benchmarking
- Run multiple iterations for statistical significance
- The T parameter may be adjusted over time as hardware improves
- VDF startup includes discriminant generation (~0.5-1s overhead)
- Verification time includes proof deserialization

## Security Considerations

The VDF parameters are chosen such that:

1. **Block production**: Even with ASIC speedup, blocks still require meaningful sequential time, preventing timestamp manipulation.

2. **Sybil resistance**: Registration VDF takes 10+ minutes, making identity spam expensive.

3. **Finality**: After N confirmations, an attacker would need N × 55 seconds of sequential work to rewrite history (not parallelizable).

| Confirmations | Reorg Time (CPU) | Reorg Time (20x ASIC) |
|---------------|------------------|------------------------|
| 1 | 55s | 2.75s |
| 6 | 5.5 min | 16.5s |
| 10 | 9.2 min | 27.5s |
| 100 | 1.5 hours | 4.6 min |
| 1000 | 15.3 hours | 45.8 min |
