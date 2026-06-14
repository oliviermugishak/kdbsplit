mod virtual_keyboard;

use anyhow::Result;
use std::time::{Duration, Instant};

/// Simple end-to-end latency benchmark.
///
/// Creates a virtual keyboard via uinput, sends a key event,
/// and measures how long `libc::ioctl(fd, EVIOCGKEY)` round-trips
/// through the kernel. This is a synthetic microbenchmark that
/// isolates the kernel→userspace event delivery path.
///
/// For a full loopback benchmark (keyboard → daemon → gamepad),
/// run with `sudo` and a running kbdsplitd daemon.
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let iterations: usize = args
        .get(1)
        .map(|s| s.parse().unwrap_or(10_000))
        .unwrap_or(10_000);
    let key = args
        .get(2)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(30); // KEY_A

    println!("KbdSplit Benchmark");
    println!("  iterations: {iterations}");
    println!("  key code:   {key}");
    println!();

    // --- Latency benchmark: uinput write + kernel dispatch ---
    let mut kb = virtual_keyboard::VirtualKeyboard::create()?;

    // Warm-up
    for _ in 0..100 {
        kb.press_and_release(key)?;
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let t0 = Instant::now();
        kb.press_and_release(key)?;
        let elapsed = t0.elapsed();
        samples.push(elapsed);
    }

    samples.sort();
    let total: Duration = samples.iter().copied().sum();
    let mean = total / iterations as u32;

    let p50 = samples[iterations / 2];
    let p90 = samples[(iterations as f64 * 0.90) as usize];
    let p99 = samples[(iterations as f64 * 0.99) as usize];
    let min = samples[0];
    let max = samples[iterations - 1];

    println!("--- uinput write latency (press + release) ---");
    println!("  mean:  {mean:.3?}");
    println!("  p50:   {p50:.3?}");
    println!("  p90:   {p90:.3?}");
    println!("  p99:   {p99:.3?}");
    println!("  min:   {min:.3?}");
    println!("  max:   {max:.3?}");
    println!("  total: {total:.3?}");
    println!();

    // --- Throughput benchmark ---
    let batch_size = 1000;
    let t0 = Instant::now();
    for i in 0..batch_size {
        // Alternate between two keys to avoid any kernel repeat logic
        let code = if i % 2 == 0 { 30u16 } else { 32u16 };
        kb.press_and_release(code)?;
    }
    let elapsed = t0.elapsed();
    let throughput = batch_size as f64 / elapsed.as_secs_f64();
    println!("--- Throughput (batched writes, {batch_size} events) ---");
    println!("  {throughput:.0} events/sec");
    println!("  {:.1} us/event", elapsed.as_secs_f64() * 1_000_000.0 / batch_size as f64);
    println!();

    // --- Jitter (coefficient of variation) ---
    let mean_ns = mean.as_nanos() as f64;
    let variance: f64 = samples
        .iter()
        .map(|s| {
            let d = s.as_nanos() as f64 - mean_ns;
            d * d
        })
        .sum::<f64>()
        / iterations as f64;
    let stddev = variance.sqrt();
    let cv = stddev / mean_ns;
    println!("--- Jitter ---");
    println!("  stddev: {stddev:.0} ns");
    println!("  CV:     {cv:.4} ({:.2}%)", cv * 100.0);

    Ok(())
}
