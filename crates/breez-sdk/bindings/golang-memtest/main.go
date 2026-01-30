package main

import (
	"context"
	"fmt"
	"net/http"
	_ "net/http/pprof"
	"os"
	"os/signal"
	"path/filepath"
	"runtime"
	"runtime/pprof"
	"syscall"
	"time"
)

func main() {
	// Parse configuration
	cfg := ParseFlags()

	if err := cfg.Validate(); err != nil {
		fmt.Fprintf(os.Stderr, "Configuration error: %v\n", err)
		os.Exit(1)
	}

	cfg.PrintConfig()

	// Create temp directory for storage
	baseDir, err := os.MkdirTemp("", "memtest-*")
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to create temp dir: %v\n", err)
		os.Exit(1)
	}
	defer os.RemoveAll(baseDir)
	fmt.Printf("Storage directory: %s\n", baseDir)

	// Start pprof server if enabled
	if cfg.PprofEnabled {
		go func() {
			addr := fmt.Sprintf(":%d", cfg.PprofPort)
			fmt.Printf("pprof server listening on http://localhost%s/debug/pprof\n", addr)
			if err := http.ListenAndServe(addr, nil); err != nil {
				fmt.Printf("pprof server error: %v\n", err)
			}
		}()
	}

	// Create context for setup (no timeout)
	setupCtx, setupCancel := context.WithCancel(context.Background())
	defer setupCancel()

	// Setup signal handling
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-sigCh
		fmt.Println("\nReceived interrupt signal, shutting down...")
		setupCancel()
	}()

	// Create faucet client
	faucet := NewFaucet(cfg.FaucetURL, cfg.FaucetUsername, cfg.FaucetPassword)
	faucetPool := NewFaucetPool(faucet)

	// Create SDK pair
	fmt.Println("\n=== Initializing SDK instances ===")
	pair, err := NewSdkPair(setupCtx, cfg, baseDir)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to create SDK pair: %v\n", err)
		os.Exit(1)
	}
	defer pair.Disconnect()

	// Create payment loop
	paymentLoop := NewPaymentLoop(pair, faucetPool, cfg)

	// Fund wallets before starting timer
	fmt.Println("\n=== Funding wallets ===")
	if err := paymentLoop.FundInitial(setupCtx); err != nil {
		fmt.Fprintf(os.Stderr, "Failed to fund wallets: %v\n", err)
		os.Exit(1)
	}

	// Now create the timed context for the actual test duration
	fmt.Printf("\n=== Starting %s test ===\n", cfg.Duration)
	ctx, cancel := context.WithTimeout(context.Background(), cfg.Duration)
	defer cancel()

	// Update signal handler to use test context
	go func() {
		<-sigCh
		cancel()
	}()

	// Create memory tracker
	tracker := NewMemoryTracker(cfg.MemoryInterval, paymentLoop.GetPaymentCount(), paymentLoop.GetListenerCount())
	if cfg.CSVFile != "" {
		tracker.SetCSVFile(cfg.CSVFile)
	}

	// Start memory tracking
	fmt.Println("\n=== Starting memory tracking ===")
	tracker.Start()
	defer tracker.Stop()

	// Run payment loop
	fmt.Println("\n=== Starting payment loop ===")
	if err := paymentLoop.Run(ctx); err != nil && err != context.DeadlineExceeded && err != context.Canceled {
		fmt.Printf("Payment loop error: %v\n", err)
	}

	// Stop payment loop
	paymentLoop.Stop()

	// Generate and print report
	report := tracker.GenerateTrendReport()
	report.PrintReport()

	// Export CSV if configured
	if err := tracker.ExportCSV(); err != nil {
		fmt.Printf("Failed to export CSV: %v\n", err)
	}

	// Dump heap profile if requested
	if cfg.HeapDumpOnExit {
		dumpHeapProfile(baseDir)
	}

	// Print event listener stats
	fmt.Println("\n=== Event Listener Stats ===")
	pair.Alice.Listener.PrintStats()
	pair.Bob.Listener.PrintStats()

	// Final verdict
	fmt.Println("\n=== Final Verdict ===")
	if report.LeakDetected {
		fmt.Println("POTENTIAL MEMORY LEAK DETECTED")
		os.Exit(1)
	} else {
		fmt.Println("No significant memory leak detected")
	}
}

// dumpHeapProfile writes a heap profile to a file.
func dumpHeapProfile(baseDir string) {
	// Force GC before dumping
	runtime.GC()

	filename := filepath.Join(baseDir, fmt.Sprintf("heap-%d.pprof", time.Now().Unix()))
	f, err := os.Create(filename)
	if err != nil {
		fmt.Printf("Failed to create heap profile: %v\n", err)
		return
	}
	defer f.Close()

	if err := pprof.WriteHeapProfile(f); err != nil {
		fmt.Printf("Failed to write heap profile: %v\n", err)
		return
	}

	fmt.Printf("Heap profile written to: %s\n", filename)
	fmt.Printf("Analyze with: go tool pprof %s\n", filename)
}
