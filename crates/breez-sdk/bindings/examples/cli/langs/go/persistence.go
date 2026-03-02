package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/tyler-smith/go-bip39"
)

const (
	phraseFileName  = "phrase"
	historyFileName = "history.txt"
)

// CliPersistence handles mnemonic and history file storage.
type CliPersistence struct {
	dataDir string
}

// GetOrCreateMnemonic reads an existing mnemonic from the data directory,
// or generates a new 12-word BIP39 mnemonic and saves it.
func (p *CliPersistence) GetOrCreateMnemonic() (string, error) {
	filename := filepath.Join(p.dataDir, phraseFileName)

	data, err := os.ReadFile(filename)
	if err == nil {
		return string(data), nil
	}
	if !os.IsNotExist(err) {
		return "", fmt.Errorf("can't read from file %s: %w", filename, err)
	}

	entropy, err := bip39.NewEntropy(128)
	if err != nil {
		return "", fmt.Errorf("failed to generate entropy: %w", err)
	}
	mnemonic, err := bip39.NewMnemonic(entropy)
	if err != nil {
		return "", fmt.Errorf("failed to generate mnemonic: %w", err)
	}

	if err := os.WriteFile(filename, []byte(mnemonic), 0600); err != nil {
		return "", fmt.Errorf("failed to write mnemonic: %w", err)
	}

	return mnemonic, nil
}

// HistoryFile returns the path to the REPL history file.
func (p *CliPersistence) HistoryFile() string {
	return filepath.Join(p.dataDir, historyFileName)
}
