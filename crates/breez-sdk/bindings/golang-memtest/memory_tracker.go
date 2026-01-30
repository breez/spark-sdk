package main

import (
	"encoding/csv"
	"fmt"
	"math"
	"os"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

// MemoryStats represents a single memory sample.
type MemoryStats struct {
	Timestamp     time.Time
	RSSBytes      uint64
	HeapAlloc     uint64
	HeapInuse     uint64
	HeapObjects   uint64
	HeapSys       uint64
	NumGoroutine  int
	PaymentCount  int64
	ListenerCount int
}

// getRSSBytes returns the current resident set size (RSS) in bytes.
func getRSSBytes() uint64 {
	switch runtime.GOOS {
	case "linux":
		data, err := os.ReadFile("/proc/self/status")
		if err != nil {
			return 0
		}
		for _, line := range strings.Split(string(data), "\n") {
			if strings.HasPrefix(line, "VmRSS:") {
				fields := strings.Fields(line)
				if len(fields) >= 2 {
					kb, _ := strconv.ParseUint(fields[1], 10, 64)
					return kb * 1024
				}
			}
		}
	case "darwin":
		// On macOS, use rusage to get RSS (simpler than mach_task_info)
		var rusage syscallRusage
		if err := getRusage(&rusage); err == nil {
			// On macOS, ru_maxrss is in bytes
			return uint64(rusage.Maxrss)
		}
	}
	return 0
}

// MemoryTracker tracks memory statistics over time.
type MemoryTracker struct {
	mu       sync.Mutex
	samples  []MemoryStats
	interval time.Duration
	stopCh   chan struct{}
	wg       sync.WaitGroup

	// External counters
	paymentCount  *int64
	listenerCount func() int

	startTime time.Time
	csvFile   string

	// Real-time CSV writing
	csvFileHandle *os.File
	csvWriter     *csv.Writer
}

// NewMemoryTracker creates a new memory tracker.
func NewMemoryTracker(interval time.Duration, paymentCount *int64, listenerCount func() int) *MemoryTracker {
	return &MemoryTracker{
		samples:       make([]MemoryStats, 0, 1000),
		interval:      interval,
		stopCh:        make(chan struct{}),
		paymentCount:  paymentCount,
		listenerCount: listenerCount,
	}
}

// SetCSVFile sets the output CSV file path.
func (m *MemoryTracker) SetCSVFile(path string) {
	m.csvFile = path
}

// Start begins the memory tracking goroutine.
func (m *MemoryTracker) Start() {
	m.startTime = time.Now()

	// Initialize CSV file for real-time writing if configured
	if m.csvFile != "" {
		f, err := os.Create(m.csvFile)
		if err != nil {
			fmt.Printf("Warning: failed to create CSV file: %v\n", err)
		} else {
			m.csvFileHandle = f
			m.csvWriter = csv.NewWriter(f)
			// Write header
			m.csvWriter.Write([]string{
				"timestamp", "elapsed_sec", "rss_bytes", "heap_alloc_bytes", "heap_inuse_bytes",
				"heap_objects", "heap_sys_bytes", "goroutines", "payments", "listeners",
			})
			m.csvWriter.Flush()
		}
	}

	// Take initial sample
	m.sample()

	m.wg.Add(1)
	go func() {
		defer m.wg.Done()
		ticker := time.NewTicker(m.interval)
		defer ticker.Stop()

		for {
			select {
			case <-ticker.C:
				m.sample()
			case <-m.stopCh:
				return
			}
		}
	}()
}

// Stop stops the memory tracking.
func (m *MemoryTracker) Stop() {
	close(m.stopCh)
	m.wg.Wait()

	// Take final sample
	m.sample()

	// Close CSV file if open
	if m.csvFileHandle != nil {
		m.csvWriter.Flush()
		m.csvFileHandle.Close()
		fmt.Printf("CSV exported to: %s\n", m.csvFile)
	}
}

// sample takes a memory sample and prints it.
func (m *MemoryTracker) sample() {
	var memStats runtime.MemStats
	runtime.ReadMemStats(&memStats)

	listenerCnt := 0
	if m.listenerCount != nil {
		listenerCnt = m.listenerCount()
	}

	stats := MemoryStats{
		Timestamp:     time.Now(),
		RSSBytes:      getRSSBytes(),
		HeapAlloc:     memStats.HeapAlloc,
		HeapInuse:     memStats.HeapInuse,
		HeapObjects:   memStats.HeapObjects,
		HeapSys:       memStats.HeapSys,
		NumGoroutine:  runtime.NumGoroutine(),
		PaymentCount:  atomic.LoadInt64(m.paymentCount),
		ListenerCount: listenerCnt,
	}

	m.mu.Lock()
	prevSample := MemoryStats{}
	if len(m.samples) > 0 {
		prevSample = m.samples[len(m.samples)-1]
	}
	m.samples = append(m.samples, stats)
	m.mu.Unlock()

	// Print current stats
	elapsed := stats.Timestamp.Sub(m.startTime)
	rssMB := float64(stats.RSSBytes) / 1024 / 1024
	heapMB := float64(stats.HeapAlloc) / 1024 / 1024

	deltaStr := ""
	if prevSample.RSSBytes > 0 {
		delta := int64(stats.RSSBytes) - int64(prevSample.RSSBytes)
		deltaMB := float64(delta) / 1024 / 1024
		if delta >= 0 {
			deltaStr = fmt.Sprintf(" (+%.2fMB)", deltaMB)
		} else {
			deltaStr = fmt.Sprintf(" (%.2fMB)", deltaMB)
		}
	}

	fmt.Printf("[%s] RSS=%.2fMB%s HeapAlloc=%.2fMB Goroutines=%d Payments=%d Listeners=%d\n",
		formatDuration(elapsed),
		rssMB,
		deltaStr,
		heapMB,
		stats.NumGoroutine,
		stats.PaymentCount,
		stats.ListenerCount,
	)

	// Write to CSV in real-time if configured
	if m.csvWriter != nil {
		m.csvWriter.Write([]string{
			stats.Timestamp.Format(time.RFC3339),
			strconv.FormatFloat(elapsed.Seconds(), 'f', 2, 64),
			strconv.FormatUint(stats.RSSBytes, 10),
			strconv.FormatUint(stats.HeapAlloc, 10),
			strconv.FormatUint(stats.HeapInuse, 10),
			strconv.FormatUint(stats.HeapObjects, 10),
			strconv.FormatUint(stats.HeapSys, 10),
			strconv.Itoa(stats.NumGoroutine),
			strconv.FormatInt(stats.PaymentCount, 10),
			strconv.Itoa(stats.ListenerCount),
		})
		m.csvWriter.Flush()
	}
}

// GetSamples returns a copy of all samples.
func (m *MemoryTracker) GetSamples() []MemoryStats {
	m.mu.Lock()
	defer m.mu.Unlock()
	result := make([]MemoryStats, len(m.samples))
	copy(result, m.samples)
	return result
}

// TrendReport contains the analysis of memory trends.
type TrendReport struct {
	Samples         []MemoryStats
	SlopeKBPerMin   float64
	RSquared        float64
	StartRSSMB      float64
	EndRSSMB        float64
	MaxRSSMB        float64
	StartHeapMB     float64
	EndHeapMB       float64
	MaxHeapMB       float64
	GoroutineStart  int
	GoroutineEnd    int
	GoroutineMax    int
	TotalPayments   int64
	LeakDetected    bool
	LeakDescription string
}

// GenerateTrendReport analyzes the samples and generates a trend report.
func (m *MemoryTracker) GenerateTrendReport() TrendReport {
	samples := m.GetSamples()
	if len(samples) < 2 {
		return TrendReport{Samples: samples}
	}

	report := TrendReport{
		Samples:        samples,
		StartRSSMB:     float64(samples[0].RSSBytes) / 1024 / 1024,
		EndRSSMB:       float64(samples[len(samples)-1].RSSBytes) / 1024 / 1024,
		StartHeapMB:    float64(samples[0].HeapAlloc) / 1024 / 1024,
		EndHeapMB:      float64(samples[len(samples)-1].HeapAlloc) / 1024 / 1024,
		GoroutineStart: samples[0].NumGoroutine,
		GoroutineEnd:   samples[len(samples)-1].NumGoroutine,
		TotalPayments:  samples[len(samples)-1].PaymentCount,
	}

	// Find max values
	for _, s := range samples {
		rssMB := float64(s.RSSBytes) / 1024 / 1024
		if rssMB > report.MaxRSSMB {
			report.MaxRSSMB = rssMB
		}
		heapMB := float64(s.HeapAlloc) / 1024 / 1024
		if heapMB > report.MaxHeapMB {
			report.MaxHeapMB = heapMB
		}
		if s.NumGoroutine > report.GoroutineMax {
			report.GoroutineMax = s.NumGoroutine
		}
	}

	// Linear regression: y = RSS (KB), x = time (minutes)
	// Calculate slope and R-squared
	n := float64(len(samples))
	var sumX, sumY, sumXY, sumX2, sumY2 float64

	startTime := samples[0].Timestamp
	for _, s := range samples {
		x := s.Timestamp.Sub(startTime).Minutes()
		y := float64(s.RSSBytes) / 1024 // KB
		sumX += x
		sumY += y
		sumXY += x * y
		sumX2 += x * x
		sumY2 += y * y
	}

	// Slope = (n*sumXY - sumX*sumY) / (n*sumX2 - sumX*sumX)
	denominator := n*sumX2 - sumX*sumX
	if denominator != 0 {
		report.SlopeKBPerMin = (n*sumXY - sumX*sumY) / denominator
	}

	// R-squared = (n*sumXY - sumX*sumY)^2 / ((n*sumX2 - sumX^2) * (n*sumY2 - sumY^2))
	numerator := n*sumXY - sumX*sumY
	denom1 := n*sumX2 - sumX*sumX
	denom2 := n*sumY2 - sumY*sumY
	if denom1 > 0 && denom2 > 0 {
		report.RSquared = (numerator * numerator) / (denom1 * denom2)
	}

	// Determine if leak detected
	// Criteria: positive slope > 100KB/min with R² > 0.7
	if report.SlopeKBPerMin > 100 && report.RSquared > 0.7 {
		report.LeakDetected = true
		report.LeakDescription = fmt.Sprintf("Consistent linear growth: +%.1f KB/min (R²=%.2f)",
			report.SlopeKBPerMin, report.RSquared)
	} else if report.GoroutineEnd > report.GoroutineStart*2 {
		report.LeakDetected = true
		report.LeakDescription = fmt.Sprintf("Goroutine count doubled: %d -> %d",
			report.GoroutineStart, report.GoroutineEnd)
	}

	return report
}

// PrintReport prints the trend report to stdout.
func (r *TrendReport) PrintReport() {
	fmt.Println("\n=== Memory Trend Report ===")
	fmt.Printf("%-10s %-12s %-12s %-10s %-14s %-12s %-10s\n",
		"Time(min)", "RSS", "HeapAlloc", "Delta", "Rate(KB/min)", "Goroutines", "Payments")
	fmt.Println("---------------------------------------------------------------------------------------")

	var prevRSS uint64
	startTime := r.Samples[0].Timestamp
	for i, s := range r.Samples {
		minutes := s.Timestamp.Sub(startTime).Minutes()
		rssMB := float64(s.RSSBytes) / 1024 / 1024
		heapMB := float64(s.HeapAlloc) / 1024 / 1024

		deltaStr := "-"
		rateStr := "-"
		if i > 0 {
			delta := int64(s.RSSBytes) - int64(prevRSS)
			deltaMB := float64(delta) / 1024 / 1024
			if delta >= 0 {
				deltaStr = fmt.Sprintf("+%.1fMB", deltaMB)
			} else {
				deltaStr = fmt.Sprintf("%.1fMB", deltaMB)
			}

			// Calculate instantaneous rate
			timeDelta := s.Timestamp.Sub(r.Samples[i-1].Timestamp).Minutes()
			if timeDelta > 0 {
				rate := float64(delta) / 1024 / timeDelta
				if rate >= 0 {
					rateStr = fmt.Sprintf("+%.0f", rate)
				} else {
					rateStr = fmt.Sprintf("%.0f", rate)
				}
			}
		}
		prevRSS = s.RSSBytes

		fmt.Printf("%-10.1f %-12.2fMB %-12.2fMB %-10s %-14s %-12d %-10d\n",
			minutes, rssMB, heapMB, deltaStr, rateStr, s.NumGoroutine, s.PaymentCount)
	}

	fmt.Println("\n--- Summary ---")
	fmt.Printf("Linear regression (RSS): %.1f KB/min (R²=%.2f)\n", r.SlopeKBPerMin, r.RSquared)
	fmt.Printf("RSS: %.2fMB -> %.2fMB (max: %.2fMB)\n", r.StartRSSMB, r.EndRSSMB, r.MaxRSSMB)
	fmt.Printf("Heap: %.2fMB -> %.2fMB (max: %.2fMB)\n", r.StartHeapMB, r.EndHeapMB, r.MaxHeapMB)
	fmt.Printf("Goroutines: %d -> %d (max: %d)\n", r.GoroutineStart, r.GoroutineEnd, r.GoroutineMax)
	fmt.Printf("Total payments: %d\n", r.TotalPayments)

	if r.LeakDetected {
		fmt.Printf("\n!!! LEAK DETECTED: %s\n", r.LeakDescription)
	} else {
		fmt.Println("\nVerdict: No significant leak detected")
	}
}

// ExportCSV is now a no-op since CSV is written in real-time.
// Kept for backwards compatibility.
func (m *MemoryTracker) ExportCSV() error {
	// CSV is now written in real-time during sampling
	// This method is kept for backwards compatibility
	return nil
}

// formatDuration formats a duration as HH:MM:SS.
func formatDuration(d time.Duration) string {
	h := int(d.Hours())
	m := int(math.Mod(d.Minutes(), 60))
	s := int(math.Mod(d.Seconds(), 60))
	return fmt.Sprintf("%02d:%02d:%02d", h, m, s)
}
