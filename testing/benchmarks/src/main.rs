//! VDF Benchmarks for DOLI
//!
//! Benchmarks for the hash-chain VDF used in block production.

use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use crypto::hash::hash;
use vdf::T_BLOCK;

#[derive(Parser)]
#[command(name = "vdf-benchmark")]
#[command(about = "Hash-chain VDF performance benchmarks for DOLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run hash-chain VDF computation benchmarks
    Compute {
        /// Number of iterations (default: 1)
        #[arg(short, long, default_value = "1")]
        iterations: u32,

        /// Custom T value (default: T_BLOCK = 800K)
        #[arg(short, long)]
        t_value: Option<u64>,
    },

    /// Run full benchmark suite
    Full,
}

fn main() {
    let cli = Cli::parse();

    println!("DOLI Hash-Chain VDF Benchmark Suite");
    println!("====================================");
    println!();

    match cli.command {
        Commands::Compute {
            iterations,
            t_value,
        } => {
            bench_hash_chain(iterations, t_value.unwrap_or(T_BLOCK));
        }
        Commands::Full => {
            run_full_suite();
        }
    }
}

/// Hash-chain VDF: state = BLAKE3(state) repeated t times
fn hash_chain_vdf(input: &crypto::Hash, t: u64) -> crypto::Hash {
    let mut state = *input;
    for _ in 0..t {
        state = hash(state.as_bytes());
    }
    state
}

/// Benchmark hash-chain VDF computation
fn bench_hash_chain(iterations: u32, t_value: u64) {
    println!("Hash-Chain VDF Benchmark");
    println!("------------------------");
    println!("T value: {t_value}");
    println!("Iterations: {iterations}");
    println!();

    let mut times: Vec<Duration> = Vec::new();

    for i in 0..iterations {
        let input = hash(format!("benchmark_input_{i}").as_bytes());

        let start = Instant::now();
        let _output = hash_chain_vdf(&input, t_value);
        let elapsed = start.elapsed();

        times.push(elapsed);
        println!(
            "  Iteration {}/{iterations}: {:.3}ms",
            i + 1,
            elapsed.as_secs_f64() * 1000.0
        );
    }

    if iterations > 1 {
        println!();
        print_stats(&times, "Hash-chain VDF");
    }
}

/// Run the full benchmark suite
fn run_full_suite() {
    println!("Running Full Benchmark Suite");
    println!("============================");
    println!();

    println!("1. Block VDF (T={T_BLOCK}, target ~55ms)");
    bench_hash_chain(5, T_BLOCK);

    println!("\n2. T-value characterization");
    for &t in &[1000, 10_000, 100_000, 500_000, 800_000] {
        let input = hash(b"characterization");
        let start = Instant::now();
        let _output = hash_chain_vdf(&input, t);
        let elapsed = start.elapsed();
        println!("  T={t:>10}: {:.3}ms", elapsed.as_secs_f64() * 1000.0);
    }

    println!("\n============================");
    println!("Benchmark suite complete");
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

    println!("{label} Statistics:");
    println!("  Min:    {:.3}ms", min.as_secs_f64() * 1000.0);
    println!("  Max:    {:.3}ms", max.as_secs_f64() * 1000.0);
    println!("  Avg:    {:.3}ms", avg_secs * 1000.0);
    println!("  StdDev: {:.3}ms", stddev * 1000.0);
}
