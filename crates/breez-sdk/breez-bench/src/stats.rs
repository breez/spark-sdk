//! Statistical analysis utilities for benchmark results.

use std::fmt::Write;
use std::time::Duration;

/// A single payment measurement with timing and metadata.
#[derive(Debug, Clone)]
pub struct PaymentMeasurement {
    /// Total duration from prepare to completion
    pub duration: Duration,
    /// Whether a payment-time swap was triggered
    pub had_swap: bool,
    /// Whether leaf optimization was cancelled during this payment
    pub had_cancellation: bool,
    /// Amount sent in satoshis (useful for future analysis by amount buckets)
    #[allow(dead_code)]
    pub amount_sats: u64,
}

/// Statistical summary of a set of duration measurements.
#[derive(Debug, Clone)]
pub struct DurationStats {
    pub count: usize,
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
    pub std_dev: Duration,
    pub p50: Duration,
    pub p75: Duration,
    pub p90: Duration,
    pub p95: Duration,
    pub p99: Duration,
}

impl DurationStats {
    /// Compute statistics from a slice of durations.
    /// Returns None if the slice is empty.
    pub fn from_durations(durations: &[Duration]) -> Option<Self> {
        if durations.is_empty() {
            return None;
        }

        let mut sorted: Vec<Duration> = durations.to_vec();
        sorted.sort();

        let count = sorted.len();
        let min = sorted[0];
        let max = sorted[count - 1];

        // Mean
        let total_nanos: u128 = sorted.iter().map(|d| d.as_nanos()).sum();
        let mean_nanos = total_nanos / count as u128;
        let mean = Duration::from_nanos(mean_nanos as u64);

        // Standard deviation
        let variance: f64 = sorted
            .iter()
            .map(|d| {
                let diff = d.as_nanos() as f64 - mean_nanos as f64;
                diff * diff
            })
            .sum::<f64>()
            / count as f64;
        let std_dev = Duration::from_nanos(variance.sqrt() as u64);

        // Percentiles
        let p50 = percentile(&sorted, 50.0);
        let p75 = percentile(&sorted, 75.0);
        let p90 = percentile(&sorted, 90.0);
        let p95 = percentile(&sorted, 95.0);
        let p99 = percentile(&sorted, 99.0);

        Some(Self {
            count,
            min,
            max,
            mean,
            std_dev,
            p50,
            p75,
            p90,
            p95,
            p99,
        })
    }

    /// Format duration as human-readable string (ms or s).
    pub fn format_duration(d: Duration) -> String {
        let ms = d.as_millis();
        if ms < 1000 {
            format!("{}ms", ms)
        } else {
            format!("{:.2}s", d.as_secs_f64())
        }
    }

    /// Print a formatted summary line.
    pub fn print_summary(&self, label: &str) {
        println!(
            "{} (n={}): p50: {}  p90: {}  p99: {}",
            label,
            self.count,
            Self::format_duration(self.p50),
            Self::format_duration(self.p90),
            Self::format_duration(self.p99),
        );
    }
}

/// Calculate percentile from a sorted slice.
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let rank = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let fraction = rank - lower as f64;

    if lower == upper {
        sorted[lower]
    } else {
        let lower_val = sorted[lower].as_nanos() as f64;
        let upper_val = sorted[upper].as_nanos() as f64;
        let interpolated = lower_val + fraction * (upper_val - lower_val);
        Duration::from_nanos(interpolated as u64)
    }
}

/// A bucket in a histogram.
#[derive(Debug, Clone)]
pub struct HistogramBucket {
    /// Lower bound (inclusive) in milliseconds
    pub lower_ms: u64,
    /// Upper bound (exclusive) in milliseconds
    pub upper_ms: u64,
    /// Number of samples in this bucket
    pub count: usize,
}

/// Histogram for visualizing duration distributions.
#[derive(Debug, Clone)]
pub struct Histogram {
    pub buckets: Vec<HistogramBucket>,
    pub total_count: usize,
}

impl Histogram {
    /// Create a histogram from durations with automatic bucket sizing.
    /// Uses ~10 buckets by default, with "nice" bucket boundaries.
    pub fn from_durations(durations: &[Duration]) -> Option<Self> {
        if durations.is_empty() {
            return None;
        }

        let min_ms = durations.iter().map(|d| d.as_millis() as u64).min()?;
        let max_ms = durations.iter().map(|d| d.as_millis() as u64).max()?;

        // Calculate a "nice" bucket width
        let range = max_ms.saturating_sub(min_ms).max(1);
        let target_buckets = 10;
        let raw_width = range / target_buckets;

        // Round to a nice number (1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, etc.)
        let bucket_width = nice_bucket_width(raw_width.max(1));

        // Align start to bucket width
        let start = (min_ms / bucket_width) * bucket_width;
        let end = ((max_ms / bucket_width) + 1) * bucket_width;

        Self::from_durations_with_buckets(durations, start, end, bucket_width)
    }

    /// Create a histogram with explicit bucket configuration.
    pub fn from_durations_with_buckets(
        durations: &[Duration],
        start_ms: u64,
        end_ms: u64,
        bucket_width_ms: u64,
    ) -> Option<Self> {
        if durations.is_empty() || bucket_width_ms == 0 {
            return None;
        }

        let mut buckets = Vec::new();
        let mut current = start_ms;

        while current < end_ms {
            let upper = current + bucket_width_ms;
            buckets.push(HistogramBucket {
                lower_ms: current,
                upper_ms: upper,
                count: 0,
            });
            current = upper;
        }

        // Count samples into buckets
        for d in durations {
            let ms = d.as_millis() as u64;
            for bucket in &mut buckets {
                if ms >= bucket.lower_ms && ms < bucket.upper_ms {
                    bucket.count += 1;
                    break;
                }
            }
            // Handle edge case: value equals max bound
            if ms >= end_ms
                && let Some(last) = buckets.last_mut()
            {
                last.count += 1;
            }
        }

        // Remove empty buckets from the edges
        while buckets.first().map(|b| b.count) == Some(0) {
            buckets.remove(0);
        }
        while buckets.last().map(|b| b.count) == Some(0) {
            buckets.pop();
        }

        Some(Self {
            total_count: durations.len(),
            buckets,
        })
    }

    /// Render the histogram as an ASCII bar chart.
    pub fn render(&self, title: &str, width: usize) -> String {
        let mut output = String::new();

        writeln!(output, "{}", title).unwrap();
        writeln!(output, "{}", "─".repeat(title.len().max(width + 20))).unwrap();

        if self.buckets.is_empty() {
            writeln!(output, "  (no data)").unwrap();
            return output;
        }

        let max_count = self.buckets.iter().map(|b| b.count).max().unwrap_or(1);

        for bucket in &self.buckets {
            let bar_len = if max_count > 0 {
                (bucket.count * width) / max_count
            } else {
                0
            };

            let percentage = (bucket.count as f64 / self.total_count as f64) * 100.0;

            // Format range label
            let range_label = format_range(bucket.lower_ms, bucket.upper_ms);

            // Build the bar
            let bar: String = "█".repeat(bar_len);
            let empty: String = " ".repeat(width.saturating_sub(bar_len));

            writeln!(
                output,
                "  {:>12} │{}{} {:>4} ({:>5.1}%)",
                range_label, bar, empty, bucket.count, percentage
            )
            .unwrap();
        }

        writeln!(output).unwrap();
        output
    }

    /// Print the histogram to stdout.
    pub fn print(&self, title: &str, width: usize) {
        print!("{}", self.render(title, width));
    }
}

/// Round a value to a "nice" number for bucket widths.
/// Returns values like 1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, etc.
fn nice_bucket_width(raw: u64) -> u64 {
    if raw == 0 {
        return 1;
    }

    // Find the order of magnitude
    let magnitude = 10u64.pow((raw as f64).log10().floor() as u32);

    // Normalize to 1-10 range
    let normalized = raw as f64 / magnitude as f64;

    // Pick the nearest "nice" number: 1, 2, 5, or 10
    let nice = if normalized <= 1.5 {
        1.0
    } else if normalized <= 3.5 {
        2.0
    } else if normalized <= 7.5 {
        5.0
    } else {
        10.0
    };

    (nice * magnitude as f64) as u64
}

/// Format a millisecond range as a human-readable string.
fn format_range(lower_ms: u64, upper_ms: u64) -> String {
    let format_ms = |ms: u64| -> String {
        if ms >= 60_000 {
            format!("{:.1}m", ms as f64 / 60_000.0)
        } else if ms >= 1000 {
            format!("{:.1}s", ms as f64 / 1000.0)
        } else {
            format!("{}ms", ms)
        }
    };

    format!("{}-{}", format_ms(lower_ms), format_ms(upper_ms))
}

/// Full benchmark results with breakdowns.
#[derive(Debug)]
pub struct BenchmarkResults {
    pub seed: u64,
    pub measurements: Vec<PaymentMeasurement>,
}

impl BenchmarkResults {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            measurements: Vec::new(),
        }
    }

    pub fn add(&mut self, measurement: PaymentMeasurement) {
        self.measurements.push(measurement);
    }

    /// Get all durations.
    pub fn all_durations(&self) -> Vec<Duration> {
        self.measurements.iter().map(|m| m.duration).collect()
    }

    /// Get durations for payments with neither swap nor cancellation.
    pub fn no_swap_no_cancel_durations(&self) -> Vec<Duration> {
        self.measurements
            .iter()
            .filter(|m| !m.had_swap && !m.had_cancellation)
            .map(|m| m.duration)
            .collect()
    }

    /// Get durations for payments with swap but no cancellation.
    pub fn swap_no_cancel_durations(&self) -> Vec<Duration> {
        self.measurements
            .iter()
            .filter(|m| m.had_swap && !m.had_cancellation)
            .map(|m| m.duration)
            .collect()
    }

    /// Get durations for payments with cancellation but no swap.
    pub fn cancel_no_swap_durations(&self) -> Vec<Duration> {
        self.measurements
            .iter()
            .filter(|m| !m.had_swap && m.had_cancellation)
            .map(|m| m.duration)
            .collect()
    }

    /// Get durations for payments with both swap and cancellation.
    pub fn swap_and_cancel_durations(&self) -> Vec<Duration> {
        self.measurements
            .iter()
            .filter(|m| m.had_swap && m.had_cancellation)
            .map(|m| m.duration)
            .collect()
    }

    /// Print full formatted report.
    pub fn print_report(&self) {
        self.print_report_with_options(true);
    }

    /// Print report with optional histogram.
    pub fn print_report_with_options(&self, show_histogram: bool) {
        println!();
        println!(
            "Payment Performance Results (seed: {}, n={})",
            self.seed,
            self.measurements.len()
        );
        println!("================================================");

        if let Some(all_stats) = DurationStats::from_durations(&self.all_durations()) {
            println!(
                "Total time:     Min: {}   Max: {}   Mean: {}   StdDev: {}",
                DurationStats::format_duration(all_stats.min),
                DurationStats::format_duration(all_stats.max),
                DurationStats::format_duration(all_stats.mean),
                DurationStats::format_duration(all_stats.std_dev),
            );
            println!(
                "  p50: {}   p75: {}   p90: {}   p95: {}   p99: {}",
                DurationStats::format_duration(all_stats.p50),
                DurationStats::format_duration(all_stats.p75),
                DurationStats::format_duration(all_stats.p90),
                DurationStats::format_duration(all_stats.p95),
                DurationStats::format_duration(all_stats.p99),
            );
        }

        println!();
        println!("Breakdown:");

        // Print swap percentage
        let total = self.measurements.len();
        let with_swap_count = self.measurements.iter().filter(|m| m.had_swap).count();
        let swap_percentage = if total > 0 {
            (with_swap_count as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "  Payments with swap: {}/{} ({:.1}%)",
            with_swap_count, total, swap_percentage
        );

        // Print cancellation percentage
        let with_cancellation_count = self
            .measurements
            .iter()
            .filter(|m| m.had_cancellation)
            .count();
        let cancellation_percentage = if total > 0 {
            (with_cancellation_count as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "  Payments with cancellation: {}/{} ({:.1}%)",
            with_cancellation_count, total, cancellation_percentage
        );
        println!();

        // Print mutually exclusive categories
        if let Some(stats) = DurationStats::from_durations(&self.no_swap_no_cancel_durations()) {
            stats.print_summary("  No swap, no cancellation  ");
        } else {
            println!("  No swap, no cancellation: (no measurements)");
        }

        if let Some(stats) = DurationStats::from_durations(&self.swap_no_cancel_durations()) {
            stats.print_summary("  Swap, no cancellation     ");
        } else {
            println!("  Swap, no cancellation: (no measurements)");
        }

        if let Some(stats) = DurationStats::from_durations(&self.cancel_no_swap_durations()) {
            stats.print_summary("  Cancellation, no swap     ");
        } else {
            println!("  Cancellation, no swap: (no measurements)");
        }

        if let Some(stats) = DurationStats::from_durations(&self.swap_and_cancel_durations()) {
            stats.print_summary("  Swap and cancellation     ");
        } else {
            println!("  Swap and cancellation: (no measurements)");
        }

        // Print histograms
        if show_histogram {
            println!();
            if let Some(histogram) = Histogram::from_durations(&self.all_durations()) {
                histogram.print("Duration Distribution (All Payments)", 40);
            }

            // Show breakdown histograms for mutually exclusive categories if there's meaningful data
            let no_swap_no_cancel = self.no_swap_no_cancel_durations();
            let swap_no_cancel = self.swap_no_cancel_durations();
            let cancel_no_swap = self.cancel_no_swap_durations();
            let swap_and_cancel = self.swap_and_cancel_durations();

            if !no_swap_no_cancel.is_empty()
                && let Some(histogram) = Histogram::from_durations(&no_swap_no_cancel)
            {
                histogram.print("Duration Distribution (No Swap, No Cancellation)", 40);
            }
            if !swap_no_cancel.is_empty()
                && let Some(histogram) = Histogram::from_durations(&swap_no_cancel)
            {
                histogram.print("Duration Distribution (Swap, No Cancellation)", 40);
            }
            if !cancel_no_swap.is_empty()
                && let Some(histogram) = Histogram::from_durations(&cancel_no_swap)
            {
                histogram.print("Duration Distribution (Cancellation, No Swap)", 40);
            }
            if !swap_and_cancel.is_empty()
                && let Some(histogram) = Histogram::from_durations(&swap_and_cancel)
            {
                histogram.print("Duration Distribution (Swap and Cancellation)", 40);
            }
        }

        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_single() {
        let durations = vec![Duration::from_millis(100)];
        assert_eq!(percentile(&durations, 50.0), Duration::from_millis(100));
    }

    #[test]
    fn test_percentile_multiple() {
        let durations: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        // Percentile uses linear interpolation: p50 of 1..=100 is index 49.5 -> interpolates to 50.5
        let p50 = percentile(&durations, 50.0);
        assert!(p50.as_millis() >= 50 && p50.as_millis() <= 51);
        let p99 = percentile(&durations, 99.0);
        assert!(p99.as_millis() >= 98 && p99.as_millis() <= 100);
    }

    #[test]
    fn test_stats_from_durations() {
        let durations: Vec<Duration> = vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
            Duration::from_millis(400),
            Duration::from_millis(500),
        ];
        let stats = DurationStats::from_durations(&durations).unwrap();
        assert_eq!(stats.count, 5);
        assert_eq!(stats.min, Duration::from_millis(100));
        assert_eq!(stats.max, Duration::from_millis(500));
        assert_eq!(stats.mean, Duration::from_millis(300));
    }

    #[test]
    fn test_nice_bucket_width() {
        // Small values (magnitude=1)
        assert_eq!(nice_bucket_width(1), 1); // 1.0 -> nice=1 -> 1
        assert_eq!(nice_bucket_width(3), 2); // 3.0 -> nice=2 -> 2
        assert_eq!(nice_bucket_width(7), 5); // 7.0 -> nice=5 -> 5
        assert_eq!(nice_bucket_width(9), 10); // 9.0 -> nice=10 -> 10

        // Values 10-99 (magnitude=10)
        assert_eq!(nice_bucket_width(15), 10); // 1.5 -> nice=1 -> 10
        assert_eq!(nice_bucket_width(25), 20); // 2.5 -> nice=2 -> 20
        assert_eq!(nice_bucket_width(45), 50); // 4.5 -> nice=5 -> 50
        assert_eq!(nice_bucket_width(80), 100); // 8.0 -> nice=10 -> 100

        // Values 100-999 (magnitude=100)
        assert_eq!(nice_bucket_width(150), 100); // 1.5 -> nice=1 -> 100
        assert_eq!(nice_bucket_width(350), 200); // 3.5 -> nice=2 -> 200
        assert_eq!(nice_bucket_width(500), 500); // 5.0 -> nice=5 -> 500
        assert_eq!(nice_bucket_width(800), 1000); // 8.0 -> nice=10 -> 1000
    }

    #[test]
    fn test_histogram_from_durations() {
        let durations: Vec<Duration> = vec![
            Duration::from_millis(100),
            Duration::from_millis(150),
            Duration::from_millis(200),
            Duration::from_millis(250),
            Duration::from_millis(300),
            Duration::from_millis(500),
            Duration::from_millis(800),
            Duration::from_millis(1000),
        ];

        let histogram = Histogram::from_durations(&durations).unwrap();
        assert_eq!(histogram.total_count, 8);
        assert!(!histogram.buckets.is_empty());

        // All samples should be counted
        let total_in_buckets: usize = histogram.buckets.iter().map(|b| b.count).sum();
        assert_eq!(total_in_buckets, 8);
    }

    #[test]
    fn test_histogram_with_explicit_buckets() {
        let durations: Vec<Duration> = vec![
            Duration::from_millis(50),
            Duration::from_millis(150),
            Duration::from_millis(250),
            Duration::from_millis(350),
            Duration::from_millis(450),
        ];

        let histogram = Histogram::from_durations_with_buckets(&durations, 0, 500, 100).unwrap();

        assert_eq!(histogram.total_count, 5);
        assert_eq!(histogram.buckets.len(), 5);

        // Check bucket counts (0-100: 1, 100-200: 1, 200-300: 1, 300-400: 1, 400-500: 1)
        assert_eq!(histogram.buckets[0].count, 1); // 50
        assert_eq!(histogram.buckets[1].count, 1); // 150
        assert_eq!(histogram.buckets[2].count, 1); // 250
        assert_eq!(histogram.buckets[3].count, 1); // 350
        assert_eq!(histogram.buckets[4].count, 1); // 450
    }

    #[test]
    fn test_histogram_render() {
        let durations: Vec<Duration> = vec![
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ];

        let histogram = Histogram::from_durations_with_buckets(&durations, 0, 400, 100).unwrap();
        let rendered = histogram.render("Test Histogram", 20);

        // Should contain title and data
        assert!(rendered.contains("Test Histogram"));
        assert!(rendered.contains("│"));
        assert!(rendered.contains("█"));
    }

    #[test]
    fn test_histogram_empty() {
        let durations: Vec<Duration> = vec![];
        assert!(Histogram::from_durations(&durations).is_none());
    }

    #[test]
    fn test_histogram_single_value() {
        let durations = vec![Duration::from_millis(500)];
        let histogram = Histogram::from_durations(&durations).unwrap();

        assert_eq!(histogram.total_count, 1);
        assert!(!histogram.buckets.is_empty());
        let total_in_buckets: usize = histogram.buckets.iter().map(|b| b.count).sum();
        assert_eq!(total_in_buckets, 1);
    }

    #[test]
    fn test_format_range() {
        assert_eq!(format_range(0, 100), "0ms-100ms");
        assert_eq!(format_range(500, 1000), "500ms-1.0s");
        assert_eq!(format_range(1000, 2000), "1.0s-2.0s");
        assert_eq!(format_range(60000, 120000), "1.0m-2.0m");
    }
}
