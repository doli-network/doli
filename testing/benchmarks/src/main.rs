//! VDF Benchmarks for DOLI
//!
//! This binary runs performance benchmarks for VDF operations
//! to characterize hardware requirements and verify timing assumptions.

use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use crypto::hash::hash;
use vdf::{compute, verify, T_BLOCK, T_REGISTER_BASE};

#[derive(Parser)]
#[command(name = "vdf-benchmark")]
#[command(about = "VDF performance benchmarks for DOLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run VDF computation benchmarks
    Compute {
        /// Number of iterations (default: 1)
        #[arg(short, long, default_value = "1")]
        iterations: u32,

        /// Custom T value (default: T_BLOCK)
        #[arg(short, long)]
        t_value: Option<u64>,
    },

    /// Run VDF verification benchmarks
    Verify {
        /// Number of iterations (default: 10)
        #[arg(short, long, default_value = "10")]
        iterations: u32,
    },

    /// Run full benchmark suite
    Full,

    /// Generate hardware characterization report
    Report {
        /// Output file path
        #[arg(short, long, default_value = "results/benchmark_report.json")]
        output: String,
    },
}

fn main() {
    let cli = Cli::parse();

    println!("DOLI VDF Benchmark Suite");
    println!("========================");
    println!();

    match cli.command {
        Commands::Compute {
            iterations,
            t_value,
        } => {
            bench_compute(iterations, t_value.unwrap_or(T_BLOCK));
        }
        Commands::Verify { iterations } => {
            bench_verify(iterations);
        }
        Commands::Full => {
            run_full_suite();
        }
        Commands::Report { output } => {
            generate_report(&output);
        }
    }
}

/// Benchmark VDF computation
fn bench_compute(iterations: u32, t_value: u64) {
    println!("VDF Computation Benchmark");
    println!("-------------------------");
    println!("T value: {}", t_value);
    println!("Iterations: {}", iterations);
    println!();

    let mut times: Vec<Duration> = Vec::new();

    for i in 0..iterations {
        println!("Iteration {}/{}", i + 1, iterations);

        // Generate random input
        let input_data = format!("benchmark_input_{}", i);
        let input = hash(input_data.as_bytes());

        // Time the computation
        let start = Instant::now();
        let (output, proof) = compute(&input, t_value).expect("VDF compute failed");
        let elapsed = start.elapsed();

        times.push(elapsed);
        println!("  Time: {:.2}s", elapsed.as_secs_f64());

        // Verify the result
        let verify_start = Instant::now();
        let valid = verify(&input, &output, &proof, t_value).is_ok();
        let verify_time = verify_start.elapsed();

        println!(
            "  Verify: {:.3}s ({})",
            verify_time.as_secs_f64(),
            if valid { "valid" } else { "INVALID" }
        );
    }

    if iterations > 1 {
        println!();
        print_stats(&times, "Computation");
    }
}

/// Benchmark VDF verification
fn bench_verify(iterations: u32) {
    println!("VDF Verification Benchmark");
    println!("--------------------------");
    println!("Iterations: {}", iterations);
    println!();

    // First, generate a valid proof to verify
    println!("Generating proof to benchmark...");
    let input_data = b"verification_benchmark_input";
    let input = hash(input_data);

    let start = Instant::now();
    let (output, proof) = compute(&input, T_BLOCK).expect("VDF compute failed");
    println!("Proof generated in {:.2}s", start.elapsed().as_secs_f64());
    println!();

    // Now benchmark verification
    let mut times: Vec<Duration> = Vec::new();

    for i in 0..iterations {
        let start = Instant::now();
        let result = verify(&input, &output, &proof, T_BLOCK);
        let elapsed = start.elapsed();

        times.push(elapsed);

        if i < 5 || i == iterations - 1 {
            println!(
                "  Iteration {}: {:.3}s ({})",
                i + 1,
                elapsed.as_secs_f64(),
                if result.is_ok() { "valid" } else { "INVALID" }
            );
        } else if i == 5 {
            println!("  ...");
        }
    }

    println!();
    print_stats(&times, "Verification");
}

/// Run the full benchmark suite
fn run_full_suite() {
    println!("Running Full Benchmark Suite");
    println!("============================");
    println!();

    // System info
    print_system_info();

    // Computation benchmarks at different T values
    println!("\n1. Block VDF Computation (T={})", T_BLOCK);
    println!("   Target: ~55 seconds");
    bench_compute(1, T_BLOCK);

    println!("\n2. Registration VDF Computation (T={})", T_REGISTER_BASE);
    println!("   Target: ~10 minutes");
    // Only run this if explicitly requested (it takes a long time)
    println!("   (Skipped - use 'compute -t {}' to run)", T_REGISTER_BASE);

    // Verification benchmarks
    println!("\n3. VDF Verification");
    println!("   Target: <1 second");
    bench_verify(5);

    // Class group operations
    println!("\n4. Class Group Operations");
    bench_class_group();

    println!("\n============================");
    println!("Benchmark suite complete");
}

/// Benchmark class group operations
fn bench_class_group() {
    println!("   Testing basic operations...");

    // Generate a discriminant
    let seed = hash(b"class_group_benchmark");
    let start = Instant::now();

    // Perform multiple squarings to characterize speed
    let iterations = 10000;
    // Note: This would need actual class group implementation access
    // For now, we just measure hashing as a baseline
    for _ in 0..iterations {
        let _ = hash(seed.as_bytes());
    }

    let elapsed = start.elapsed();
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("   Hash operations: {:.0}/sec", ops_per_sec);
    println!("   (Class group ops would be slower - requires VDF impl access)");
}

/// Generate a benchmark report
fn generate_report(output: &str) {
    use serde_json::json;
    use std::fs;

    println!("Generating Benchmark Report");
    println!("---------------------------");
    println!();

    // Run benchmarks and collect data
    let mut results = vec![];

    // Block VDF
    println!("Running block VDF benchmark...");
    let input = hash(b"report_benchmark");
    let start = Instant::now();
    let (output_vdf, proof) = compute(&input, T_BLOCK).expect("VDF compute failed");
    let compute_time = start.elapsed();

    let start = Instant::now();
    let _ = verify(&input, &output_vdf, &proof, T_BLOCK);
    let verify_time = start.elapsed();

    results.push(json!({
        "name": "block_vdf",
        "t_value": T_BLOCK,
        "compute_seconds": compute_time.as_secs_f64(),
        "verify_seconds": verify_time.as_secs_f64(),
    }));

    // Create report
    let report = json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "system": get_system_info(),
        "results": results,
        "summary": {
            "block_vdf_compute_target": 55,
            "block_vdf_verify_target": 1,
            "registration_vdf_target": 600,
        }
    });

    // Write to file
    if let Some(parent) = std::path::Path::new(output).parent() {
        let _ = fs::create_dir_all(parent);
    }

    match fs::write(output, serde_json::to_string_pretty(&report).unwrap()) {
        Ok(_) => println!("Report written to: {}", output),
        Err(e) => println!("Error writing report: {}", e),
    }
}

/// Print system information
fn print_system_info() {
    println!("System Information");
    println!("------------------");

    #[cfg(target_os = "linux")]
    {
        // Try to get CPU info
        if let Ok(cpu_info) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in cpu_info.lines() {
                if line.starts_with("model name") {
                    println!(
                        "CPU: {}",
                        line.split(':').nth(1).unwrap_or("Unknown").trim()
                    );
                    break;
                }
            }
        }

        // Memory info
        if let Ok(mem_info) = std::fs::read_to_string("/proc/meminfo") {
            for line in mem_info.lines() {
                if line.starts_with("MemTotal") {
                    let mem_kb: u64 = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    println!("Memory: {} GB", mem_kb / 1024 / 1024);
                    break;
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        println!("CPU: (detection not available on this platform)");
        println!("Memory: (detection not available on this platform)");
    }

    println!("OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
}

/// Get system info as JSON-compatible structure
#[allow(unused_mut)]
fn get_system_info() -> serde_json::Value {
    use serde_json::json;

    let mut cpu = "Unknown".to_string();
    let mut memory_gb = 0u64;

    #[cfg(target_os = "linux")]
    {
        if let Ok(cpu_info) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in cpu_info.lines() {
                if line.starts_with("model name") {
                    cpu = line
                        .split(':')
                        .nth(1)
                        .unwrap_or("Unknown")
                        .trim()
                        .to_string();
                    break;
                }
            }
        }

        if let Ok(mem_info) = std::fs::read_to_string("/proc/meminfo") {
            for line in mem_info.lines() {
                if line.starts_with("MemTotal") {
                    let mem_kb: u64 = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    memory_gb = mem_kb / 1024 / 1024;
                    break;
                }
            }
        }
    }

    json!({
        "cpu": cpu,
        "memory_gb": memory_gb,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    })
}

/// Print statistics for timing data
fn print_stats(times: &[Duration], label: &str) {
    if times.is_empty() {
        return;
    }

    let total: Duration = times.iter().sum();
    let avg = total / times.len() as u32;
    let min = times.iter().min().unwrap();
    let max = times.iter().max().unwrap();

    // Standard deviation
    let avg_secs = avg.as_secs_f64();
    let variance: f64 = times
        .iter()
        .map(|t| {
            let diff = t.as_secs_f64() - avg_secs;
            diff * diff
        })
        .sum::<f64>()
        / times.len() as f64;
    let stddev = variance.sqrt();

    println!("{} Statistics:", label);
    println!("  Min:    {:.3}s", min.as_secs_f64());
    println!("  Max:    {:.3}s", max.as_secs_f64());
    println!("  Avg:    {:.3}s", avg_secs);
    println!("  StdDev: {:.3}s", stddev);
}
