package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sync"

	sdk "breez_sdk_spark_go/breez_sdk_spark"
)

// unwrapSdkError works around a uniffi-bindgen-go bug where a nil *SdkError
// is returned as a non-nil error interface. This checks if the error is a
// nil *SdkError and returns nil in that case.
func unwrapSdkError(err error) error {
	if err == nil {
		return nil
	}
	if sdkErr, ok := err.(*sdk.SdkError); ok && sdkErr == nil {
		return nil
	}
	return err
}

// SdkInstance wraps an SDK instance with its associated resources.
type SdkInstance struct {
	SDK         *sdk.BreezSdk
	Listener    *TestEventListener
	ListenerID  string
	StorageDir  string
	Name        string
	SparkAddr   string
	BitcoinAddr string // Bitcoin address for faucet funding

	mu sync.Mutex
}

// NewSdkInstance creates and connects a new SDK instance.
func NewSdkInstance(ctx context.Context, name string, seed [32]byte, baseDir string) (*SdkInstance, error) {
	storageDir := filepath.Join(baseDir, name)
	if err := os.MkdirAll(storageDir, 0755); err != nil {
		return nil, fmt.Errorf("failed to create storage dir: %w", err)
	}

	// Create config for regtest
	config := sdk.DefaultConfig(sdk.NetworkRegtest)

	// Disable real-time sync for testing
	config.SyncIntervalSecs = 5 // Faster sync for tests
	config.PreferSparkOverLightning = true

	// Create seed from entropy
	seedObj := sdk.SeedEntropy{Field0: seed[:]}

	// Connect
	request := sdk.ConnectRequest{
		Config:     config,
		Seed:       seedObj,
		StorageDir: storageDir,
	}

	sdkInstance, err := sdk.Connect(request)
	if err := unwrapSdkError(err); err != nil {
		return nil, fmt.Errorf("failed to connect SDK: %w", err)
	}

	// Create and register event listener
	listener := NewTestEventListener(name)
	listenerID := sdkInstance.AddEventListener(listener)

	instance := &SdkInstance{
		SDK:        sdkInstance,
		Listener:   listener,
		ListenerID: listenerID,
		StorageDir: storageDir,
		Name:       name,
	}

	// Get Spark address via ReceivePayment
	receiveResp, err := sdkInstance.ReceivePayment(sdk.ReceivePaymentRequest{
		PaymentMethod: sdk.ReceivePaymentMethodSparkAddress{},
	})
	if err := unwrapSdkError(err); err != nil {
		// Clean up on failure
		sdkInstance.Disconnect()
		return nil, fmt.Errorf("failed to get Spark address: %w", err)
	}
	instance.SparkAddr = receiveResp.PaymentRequest

	// Get Bitcoin address for faucet funding
	btcReceiveResp, err := sdkInstance.ReceivePayment(sdk.ReceivePaymentRequest{
		PaymentMethod: sdk.ReceivePaymentMethodBitcoinAddress{},
	})
	if err := unwrapSdkError(err); err != nil {
		// Clean up on failure
		sdkInstance.Disconnect()
		return nil, fmt.Errorf("failed to get Bitcoin address: %w", err)
	}
	instance.BitcoinAddr = btcReceiveResp.PaymentRequest

	fmt.Printf("[%s] Connected, Spark: %s, Bitcoin: %s\n", name, truncateAddress(instance.SparkAddr), truncateAddress(instance.BitcoinAddr))

	return instance, nil
}

// Disconnect cleanly disconnects the SDK instance.
func (s *SdkInstance) Disconnect() error {
	s.mu.Lock()
	defer s.mu.Unlock()

	if s.SDK == nil {
		return nil
	}

	// Remove listener first
	if s.ListenerID != "" {
		s.SDK.RemoveEventListener(s.ListenerID)
		s.ListenerID = ""
	}

	// Disconnect SDK
	if err := unwrapSdkError(s.SDK.Disconnect()); err != nil {
		return fmt.Errorf("failed to disconnect %s: %w", s.Name, err)
	}

	fmt.Printf("[%s] Disconnected\n", s.Name)
	s.SDK = nil

	return nil
}

// Reconnect disconnects and reconnects the SDK.
func (s *SdkInstance) Reconnect(ctx context.Context, seed [32]byte) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	// Disconnect current instance
	if s.SDK != nil {
		if s.ListenerID != "" {
			s.SDK.RemoveEventListener(s.ListenerID)
		}
		if err := unwrapSdkError(s.SDK.Disconnect()); err != nil {
			fmt.Printf("[%s] Warning: disconnect error: %v\n", s.Name, err)
		}
	}

	// Small delay to allow cleanup
	// Note: In a real scenario, we might want to wait for background tasks to stop

	// Reconnect
	config := sdk.DefaultConfig(sdk.NetworkRegtest)
	config.SyncIntervalSecs = 5
	config.PreferSparkOverLightning = true

	seedObj := sdk.SeedEntropy{Field0: seed[:]}

	request := sdk.ConnectRequest{
		Config:     config,
		Seed:       seedObj,
		StorageDir: s.StorageDir,
	}

	sdkInstance, err := sdk.Connect(request)
	if err := unwrapSdkError(err); err != nil {
		return fmt.Errorf("failed to reconnect %s: %w", s.Name, err)
	}

	s.SDK = sdkInstance

	// Re-register listener
	s.Listener = NewTestEventListener(s.Name)
	s.ListenerID = sdkInstance.AddEventListener(s.Listener)

	// Get Spark address (should be the same)
	receiveResp, err := sdkInstance.ReceivePayment(sdk.ReceivePaymentRequest{
		PaymentMethod: sdk.ReceivePaymentMethodSparkAddress{},
	})
	if err := unwrapSdkError(err); err != nil {
		return fmt.Errorf("failed to get Spark address: %w", err)
	}
	s.SparkAddr = receiveResp.PaymentRequest

	fmt.Printf("[%s] Reconnected\n", s.Name)

	return nil
}

// GetBalance returns the current balance.
func (s *SdkInstance) GetBalance() (uint64, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if s.SDK == nil {
		return 0, fmt.Errorf("SDK not connected")
	}

	info, err := s.SDK.GetInfo(sdk.GetInfoRequest{})
	if err := unwrapSdkError(err); err != nil {
		return 0, err
	}

	return info.BalanceSats, nil
}

// SdkPair manages two SDK instances for payment testing.
type SdkPair struct {
	Alice       *SdkInstance
	Bob         *SdkInstance
	ExtraAlices []*SdkInstance
	ExtraBobs   []*SdkInstance
	cfg         *Config
}

// NewSdkPair creates a new pair of SDK instances.
func NewSdkPair(ctx context.Context, cfg *Config, baseDir string) (*SdkPair, error) {
	alice, err := NewSdkInstance(ctx, "alice", cfg.AliceSeed, baseDir)
	if err != nil {
		return nil, fmt.Errorf("failed to create Alice: %w", err)
	}

	bob, err := NewSdkInstance(ctx, "bob", cfg.BobSeed, baseDir)
	if err != nil {
		alice.Disconnect()
		return nil, fmt.Errorf("failed to create Bob: %w", err)
	}

	pair := &SdkPair{
		Alice: alice,
		Bob:   bob,
		cfg:   cfg,
	}

	// Create extra instances (same seeds, different storage dirs)
	for i := 0; i < cfg.ExtraInstances; i++ {
		extraAlice, err := NewSdkInstance(ctx, fmt.Sprintf("extra-alice-%d", i), cfg.AliceSeed, baseDir)
		if err != nil {
			pair.Disconnect()
			return nil, fmt.Errorf("failed to create extra-alice-%d: %w", i, err)
		}
		pair.ExtraAlices = append(pair.ExtraAlices, extraAlice)

		extraBob, err := NewSdkInstance(ctx, fmt.Sprintf("extra-bob-%d", i), cfg.BobSeed, baseDir)
		if err != nil {
			pair.Disconnect()
			return nil, fmt.Errorf("failed to create extra-bob-%d: %w", i, err)
		}
		pair.ExtraBobs = append(pair.ExtraBobs, extraBob)
	}

	return pair, nil
}

// Disconnect disconnects all SDK instances (extras first, then base).
func (p *SdkPair) Disconnect() error {
	var errs []error

	// Disconnect extras first
	for _, extra := range p.ExtraAlices {
		if err := extra.Disconnect(); err != nil {
			errs = append(errs, err)
		}
	}
	for _, extra := range p.ExtraBobs {
		if err := extra.Disconnect(); err != nil {
			errs = append(errs, err)
		}
	}

	// Disconnect base instances
	if err := p.Alice.Disconnect(); err != nil {
		errs = append(errs, err)
	}

	if err := p.Bob.Disconnect(); err != nil {
		errs = append(errs, err)
	}

	if len(errs) > 0 {
		return fmt.Errorf("disconnect errors: %v", errs)
	}

	return nil
}

// Reconnect reconnects all SDK instances (base and extras).
func (p *SdkPair) Reconnect(ctx context.Context, aliceSeed, bobSeed [32]byte) error {
	// Reconnect base instances
	if err := p.Alice.Reconnect(ctx, aliceSeed); err != nil {
		return err
	}

	if err := p.Bob.Reconnect(ctx, bobSeed); err != nil {
		return err
	}

	// Reconnect extras
	for _, extra := range p.ExtraAlices {
		if err := extra.Reconnect(ctx, aliceSeed); err != nil {
			return err
		}
	}
	for _, extra := range p.ExtraBobs {
		if err := extra.Reconnect(ctx, bobSeed); err != nil {
			return err
		}
	}

	return nil
}
