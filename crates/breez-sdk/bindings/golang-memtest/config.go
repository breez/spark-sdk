package main

import (
	"encoding/hex"
	"flag"
	"fmt"
	"os"
	"strings"
	"time"
)

// PaymentType represents the type of payment to use in tests.
type PaymentType int

const (
	PaymentTypeSpark PaymentType = iota
	PaymentTypeLightning
	PaymentTypeBoth
)

func (p PaymentType) String() string {
	switch p {
	case PaymentTypeSpark:
		return "spark"
	case PaymentTypeLightning:
		return "lightning"
	case PaymentTypeBoth:
		return "both"
	default:
		return "unknown"
	}
}

func ParsePaymentType(s string) (PaymentType, error) {
	switch strings.ToLower(s) {
	case "spark":
		return PaymentTypeSpark, nil
	case "lightning", "ln":
		return PaymentTypeLightning, nil
	case "both", "all":
		return PaymentTypeBoth, nil
	default:
		return PaymentTypeSpark, fmt.Errorf("invalid payment type: %s (use spark, lightning, or both)", s)
	}
}

// Config holds all configuration for the memory leak test.
type Config struct {
	// Test duration
	Duration time.Duration

	// Payment interval
	PaymentInterval time.Duration

	// Memory sampling interval
	MemoryInterval time.Duration

	// Amount per payment in sats
	AmountSats uint64

	// Payment type (spark, lightning, or both)
	PaymentType PaymentType

	// Reconnect cycle settings
	ReconnectCycles bool
	ReconnectEvery  int

	// Listener churn settings
	ListenerChurn bool

	// Frequent sync calls
	FrequentSync bool

	// Payment history queries
	PaymentHistoryQueries bool
	PaymentHistoryLimit   uint32 // 0 = unlimited

	// Memory management options (for debugging)
	DestroyResponses bool // Call Destroy() on ListPayments response
	ForceGC          bool // Force GC after each payment cycle

	// Extra instances (same seeds as alice/bob, multiple SDK connections to same wallet)
	ExtraInstances int

	// Profiling
	PprofEnabled bool
	PprofPort    int

	// Output settings
	HeapDumpOnExit bool
	CSVFile        string

	// Faucet settings
	FaucetURL      string
	FaucetUsername string
	FaucetPassword string

	// Seed bytes for Alice and Bob (deterministic for reproducibility)
	AliceSeed [32]byte
	BobSeed   [32]byte
}

// DefaultConfig returns a Config with sensible defaults.
func DefaultConfig() *Config {
	return &Config{
		Duration:        1 * time.Hour,
		PaymentInterval: 5 * time.Second,
		MemoryInterval:  30 * time.Second,
		AmountSats:      1000,
		PaymentType:     PaymentTypeSpark,
		ReconnectCycles: false,
		ReconnectEvery:  100,
		ListenerChurn:   false,
		ExtraInstances:  0,
		PprofEnabled:    false,
		PprofPort:       6060,
		HeapDumpOnExit:  false,
		CSVFile:         "",
		FaucetURL:       "https://api.lightspark.com/graphql/spark/rc",
		FaucetUsername:  os.Getenv("FAUCET_USERNAME"),
		FaucetPassword:  os.Getenv("FAUCET_PASSWORD"),
		AliceSeed:       parseSeedFromEnv("ALICE_SEED"),
		BobSeed:         parseSeedFromEnv("BOB_SEED"),
	}
}

// parseSeedFromEnv parses a 32-byte seed from a hex-encoded environment variable.
// Returns zero seed if not set (will be validated later).
func parseSeedFromEnv(envVar string) [32]byte {
	var seed [32]byte
	hexStr := os.Getenv(envVar)
	if hexStr == "" {
		return seed
	}
	decoded, err := hex.DecodeString(hexStr)
	if err != nil || len(decoded) != 32 {
		return seed
	}
	copy(seed[:], decoded)
	return seed
}

// ParseFlags parses command-line flags into the Config.
func ParseFlags() *Config {
	cfg := DefaultConfig()

	var paymentTypeStr string

	flag.DurationVar(&cfg.Duration, "duration", cfg.Duration, "Test duration")
	flag.DurationVar(&cfg.PaymentInterval, "interval", cfg.PaymentInterval, "Payment interval")
	flag.DurationVar(&cfg.MemoryInterval, "mem-interval", cfg.MemoryInterval, "Memory sampling interval")
	flag.Uint64Var(&cfg.AmountSats, "amount", cfg.AmountSats, "Satoshis per payment")
	flag.StringVar(&paymentTypeStr, "payment-type", "spark", "Payment type: spark, lightning, or both")
	flag.BoolVar(&cfg.ReconnectCycles, "reconnect-cycles", cfg.ReconnectCycles, "Enable disconnect/reconnect cycles")
	flag.IntVar(&cfg.ReconnectEvery, "reconnect-every", cfg.ReconnectEvery, "Payments between reconnects")
	flag.BoolVar(&cfg.ListenerChurn, "listener-churn", cfg.ListenerChurn, "Enable listener add/remove churn")
	flag.BoolVar(&cfg.FrequentSync, "frequent-sync", cfg.FrequentSync, "Call sync_wallet on every payment cycle")
	flag.BoolVar(&cfg.PaymentHistoryQueries, "payment-history", cfg.PaymentHistoryQueries, "Query payment history on every payment cycle")
	flag.Func("payment-history-limit", "Limit for payment history queries (0 = unlimited)", func(s string) error {
		var v uint64
		if _, err := fmt.Sscanf(s, "%d", &v); err != nil {
			return err
		}
		cfg.PaymentHistoryLimit = uint32(v)
		return nil
	})
	flag.BoolVar(&cfg.DestroyResponses, "destroy-responses", cfg.DestroyResponses, "Call Destroy() on ListPayments responses")
	flag.BoolVar(&cfg.ForceGC, "force-gc", cfg.ForceGC, "Force GC after each payment cycle")
	flag.IntVar(&cfg.ExtraInstances, "extra-instances", cfg.ExtraInstances, "Extra SDK instances (same seeds as alice/bob)")
	flag.BoolVar(&cfg.PprofEnabled, "pprof", cfg.PprofEnabled, "Enable pprof HTTP endpoint")
	flag.IntVar(&cfg.PprofPort, "pprof-port", cfg.PprofPort, "Port for pprof endpoint")
	flag.BoolVar(&cfg.HeapDumpOnExit, "heap-dump", cfg.HeapDumpOnExit, "Dump heap profile on exit")
	flag.StringVar(&cfg.CSVFile, "csv", cfg.CSVFile, "Export time-series to CSV file")
	flag.StringVar(&cfg.FaucetURL, "faucet-url", cfg.FaucetURL, "Faucet GraphQL URL")

	flag.Parse()

	// Parse payment type
	if pt, err := ParsePaymentType(paymentTypeStr); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	} else {
		cfg.PaymentType = pt
	}

	// Override faucet credentials from env if set
	if u := os.Getenv("FAUCET_USERNAME"); u != "" {
		cfg.FaucetUsername = u
	}
	if p := os.Getenv("FAUCET_PASSWORD"); p != "" {
		cfg.FaucetPassword = p
	}

	return cfg
}

// Validate checks the configuration for errors.
func (c *Config) Validate() error {
	if c.Duration <= 0 {
		return fmt.Errorf("duration must be positive")
	}
	if c.PaymentInterval <= 0 {
		return fmt.Errorf("payment interval must be positive")
	}
	if c.MemoryInterval <= 0 {
		return fmt.Errorf("memory interval must be positive")
	}
	if c.AmountSats == 0 {
		return fmt.Errorf("amount must be > 0")
	}
	if c.ReconnectCycles && c.ReconnectEvery <= 0 {
		return fmt.Errorf("reconnect-every must be positive when reconnect-cycles enabled")
	}
	// Check seeds are provided
	var zeroSeed [32]byte
	if c.AliceSeed == zeroSeed {
		return fmt.Errorf("ALICE_SEED env var required (64 hex chars)")
	}
	if c.BobSeed == zeroSeed {
		return fmt.Errorf("BOB_SEED env var required (64 hex chars)")
	}
	return nil
}

// PrintConfig outputs the current configuration.
func (c *Config) PrintConfig() {
	fmt.Println("=== Memory Leak Test Configuration ===")
	fmt.Printf("Duration:         %v\n", c.Duration)
	fmt.Printf("Payment interval: %v\n", c.PaymentInterval)
	fmt.Printf("Memory interval:  %v\n", c.MemoryInterval)
	fmt.Printf("Amount:           %d sats\n", c.AmountSats)
	fmt.Printf("Payment type:     %s\n", c.PaymentType)
	fmt.Printf("Reconnect cycles: %v", c.ReconnectCycles)
	if c.ReconnectCycles {
		fmt.Printf(" (every %d payments)", c.ReconnectEvery)
	}
	fmt.Println()
	fmt.Printf("Listener churn:   %v\n", c.ListenerChurn)
	fmt.Printf("Frequent sync:    %v\n", c.FrequentSync)
	fmt.Printf("Payment history:  %v", c.PaymentHistoryQueries)
	if c.PaymentHistoryQueries {
		if c.PaymentHistoryLimit > 0 {
			fmt.Printf(" (limit %d)", c.PaymentHistoryLimit)
		} else {
			fmt.Printf(" (unlimited)")
		}
	}
	fmt.Println()
	fmt.Printf("Destroy responses: %v\n", c.DestroyResponses)
	fmt.Printf("Force GC:         %v\n", c.ForceGC)
	fmt.Printf("Extra instances:  %d\n", c.ExtraInstances)
	fmt.Printf("Pprof enabled:    %v", c.PprofEnabled)
	if c.PprofEnabled {
		fmt.Printf(" (port %d)", c.PprofPort)
	}
	fmt.Println()
	fmt.Printf("Heap dump on exit: %v\n", c.HeapDumpOnExit)
	if c.CSVFile != "" {
		fmt.Printf("CSV output:       %s\n", c.CSVFile)
	}
	fmt.Println("======================================")
}
