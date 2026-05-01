package main

import (
	"bufio"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// PasskeyProvider identifies which PRF provider to use.
type PasskeyProvider int

const (
	PasskeyProviderFile PasskeyProvider = iota
	PasskeyProviderYubiKey
	PasskeyProviderFido2
)

// PasskeyConfig holds passkey-related CLI options.
type PasskeyConfig struct {
	Provider    PasskeyProvider
	Label       *string
	ListLabels  bool
	StoreLabel  bool
	RpID        *string
}

// parsePasskeyProvider parses a provider name string into a PasskeyProvider.
func parsePasskeyProvider(s string) (PasskeyProvider, error) {
	switch strings.ToLower(s) {
	case "file":
		return PasskeyProviderFile, nil
	case "yubikey":
		return PasskeyProviderYubiKey, nil
	case "fido2":
		return PasskeyProviderFido2, nil
	default:
		return 0, fmt.Errorf("invalid passkey provider '%s' (valid: file, yubikey, fido2)", s)
	}
}

// ---------------------------------------------------------------------------
// File-based PRF provider
// ---------------------------------------------------------------------------

const secretFileName = "seedless-restore-secret"

// FilePrfProvider implements breez_sdk_spark.PrfProvider using
// HMAC-SHA256 with a secret stored in a file.
type FilePrfProvider struct {
	secret [32]byte
}

// NewFilePrfProvider creates a FilePrfProvider. If the secret file doesn't
// exist a random 32-byte secret is generated and persisted.
func NewFilePrfProvider(dataDir string) (*FilePrfProvider, error) {
	secretPath := filepath.Join(dataDir, secretFileName)

	data, err := os.ReadFile(secretPath)
	if err == nil {
		if len(data) != 32 {
			return nil, fmt.Errorf("invalid secret file: expected 32 bytes, got %d", len(data))
		}
		var secret [32]byte
		copy(secret[:], data)
		return &FilePrfProvider{secret: secret}, nil
	}
	if !os.IsNotExist(err) {
		return nil, fmt.Errorf("failed to read secret file: %w", err)
	}

	// Generate new random secret
	var secret [32]byte
	if _, err := rand.Read(secret[:]); err != nil {
		return nil, fmt.Errorf("failed to generate secret: %w", err)
	}

	if err := os.MkdirAll(dataDir, 0755); err != nil {
		return nil, fmt.Errorf("failed to create data directory: %w", err)
	}
	if err := os.WriteFile(secretPath, secret[:], 0600); err != nil {
		return nil, fmt.Errorf("failed to write secret file: %w", err)
	}

	return &FilePrfProvider{secret: secret}, nil
}

func (f *FilePrfProvider) DerivePrfSeed(salt string) ([]byte, error) {
	mac := hmac.New(sha256.New, f.secret[:])
	mac.Write([]byte(salt))
	return mac.Sum(nil), nil
}

func (f *FilePrfProvider) IsPrfAvailable() (bool, error) {
	return true, nil
}

func (f *FilePrfProvider) CheckDomainAssociation() (breez_sdk_spark.DomainAssociation, error) {
	return breez_sdk_spark.DomainAssociationSkipped{
		Reason: "FilePrfProvider does not verify domain association",
	}, nil
}

// ---------------------------------------------------------------------------
// Stub providers for hardware-dependent backends
// ---------------------------------------------------------------------------

type notYetSupportedProvider struct {
	name string
}

func (p *notYetSupportedProvider) DerivePrfSeed(_ string) ([]byte, error) {
	return nil, fmt.Errorf("%s passkey provider is not yet supported in the Go CLI", p.name)
}

func (p *notYetSupportedProvider) IsPrfAvailable() (bool, error) {
	return false, fmt.Errorf("%s passkey provider is not yet supported in the Go CLI", p.name)
}

func (p *notYetSupportedProvider) CheckDomainAssociation() (breez_sdk_spark.DomainAssociation, error) {
	return breez_sdk_spark.DomainAssociationSkipped{
		Reason: fmt.Sprintf("%s does not verify domain association", p.name),
	}, nil
}

// ---------------------------------------------------------------------------
// Passkey seed resolution (orchestration)
// ---------------------------------------------------------------------------

// resolvePasskeySeed derives a wallet seed using the given PRF provider,
// matching the Rust CLI's resolve_passkey_seed logic.
func resolvePasskeySeed(
	provider breez_sdk_spark.PrfProvider,
	breezAPIKey *string,
	label *string,
	listLabels bool,
	storeLabel bool,
) (breez_sdk_spark.Seed, error) {
	relayConfig := &breez_sdk_spark.NostrRelayConfig{
		BreezApiKey: breezAPIKey,
	}
	passkey := breez_sdk_spark.NewPasskey(provider, relayConfig)

	// --store-label: publish to Nostr
	if storeLabel && label != nil {
		fmt.Printf("Publishing label '%s' to Nostr...\n", *label)
		if err := liftError(passkey.StoreLabel(*label)); err != nil {
			return nil, fmt.Errorf("failed to store label: %w", err)
		}
		fmt.Printf("Label '%s' published successfully.\n", *label)
	}

	// --list-labels: query Nostr and prompt user to select
	resolvedName := label
	if listLabels {
		fmt.Println("Querying Nostr for available labels...")
		labels, err := passkey.ListLabels()
		if err = liftError(err); err != nil {
			return nil, fmt.Errorf("failed to list labels: %w", err)
		}

		if len(labels) == 0 {
			return nil, fmt.Errorf("no labels found on Nostr for this identity")
		}

		fmt.Println("Available labels:")
		for i, name := range labels {
			fmt.Printf("  %d: %s\n", i+1, name)
		}

		fmt.Printf("Select label (1-%d): ", len(labels))
		reader := bufio.NewReader(os.Stdin)
		input, _ := reader.ReadString('\n')
		idx, err := strconv.Atoi(strings.TrimSpace(input))
		if err != nil {
			return nil, fmt.Errorf("invalid selection")
		}
		if idx < 1 || idx > len(labels) {
			return nil, fmt.Errorf("selection out of range")
		}

		selected := labels[idx-1]
		resolvedName = &selected
	}

	wallet, err := passkey.GetWallet(resolvedName)
	if err = liftError(err); err != nil {
		return nil, fmt.Errorf("failed to derive wallet: %w", err)
	}
	return wallet.Seed, nil
}

// buildPrfProvider creates a PrfProvider for the given provider type.
func buildPrfProvider(provider PasskeyProvider, dataDir string) (breez_sdk_spark.PrfProvider, error) {
	switch provider {
	case PasskeyProviderFile:
		return NewFilePrfProvider(dataDir)
	case PasskeyProviderYubiKey:
		return &notYetSupportedProvider{name: "YubiKey"}, nil
	case PasskeyProviderFido2:
		return &notYetSupportedProvider{name: "FIDO2"}, nil
	default:
		return nil, fmt.Errorf("unknown passkey provider")
	}
}
