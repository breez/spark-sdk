package main

import (
	"flag"
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// expandPath expands a leading ~/ to the user's home directory.
func expandPath(path string) string {
	if strings.HasPrefix(path, "~/") {
		home, err := os.UserHomeDir()
		if err != nil {
			log.Fatalf("Could not find home directory: %v", err)
		}
		return filepath.Join(home, path[2:])
	}
	return path
}

// splitArgs splits a command line into arguments, handling double-quoted strings.
func splitArgs(line string) []string {
	var args []string
	var current strings.Builder
	inQuote := false

	for _, r := range line {
		switch {
		case r == '"':
			inQuote = !inQuote
		case r == ' ' && !inQuote:
			if current.Len() > 0 {
				args = append(args, current.String())
				current.Reset()
			}
		default:
			current.WriteRune(r)
		}
	}
	if current.Len() > 0 {
		args = append(args, current.String())
	}
	return args
}

// CliEventListener logs SDK events as JSON.
type CliEventListener struct{}

func (CliEventListener) OnEvent(event breez_sdk_spark.SdkEvent) {
	log.Printf("Event: %s", serialize(event))
}

// liftError converts a typed nil *SdkError (Go UniFFI binding quirk) to an
// untyped nil so that `err != nil` behaves correctly.  The generated bindings
// return *SdkError which, even on success, wraps as a non-nil error interface
// containing a nil pointer.
func liftError(err error) error {
	if err == nil {
		return nil
	}
	if sdkErr, ok := err.(*breez_sdk_spark.SdkError); ok {
		return sdkErr.AsError()
	}
	return err
}

func main() {
	// CLI flags
	dataDir := flag.String("d", "./.data", "Path to the data directory")
	flag.StringVar(dataDir, "data-dir", "./.data", "Path to the data directory")
	network := flag.String("network", "regtest", "Network to use (regtest or mainnet)")
	accountNumber := flag.String("account-number", "", "Account number for the Spark signer")
	postgresConnectionString := flag.String("postgres-connection-string", "", "PostgreSQL connection string (uses SQLite by default)")
	stableBalanceTokenIdentifier := flag.String("stable-balance-token-identifier", "", "Stable balance token identifier")
	stableBalanceThreshold := flag.Uint64("stable-balance-threshold", 0, "Stable balance threshold in sats")
	passkeyProviderStr := flag.String("passkey", "", "Use passkey with PRF provider (file, yubikey, or fido2)")
	label := flag.String("label", "", "Label for seed derivation (requires --passkey)")
	listLabels := flag.Bool("list-labels", false, "List and select from labels published to Nostr (requires --passkey)")
	storeLabel := flag.Bool("store-label", false, "Publish the label to Nostr (requires --passkey and --label)")
	_ = flag.String("rpid", "", "Relying party ID for FIDO2 provider (requires --passkey)")
	flag.Parse()

	resolvedDir := expandPath(*dataDir)
	if err := os.MkdirAll(resolvedDir, 0755); err != nil {
		log.Fatalf("Failed to create data directory: %v", err)
	}

	// Parse network
	var networkEnum breez_sdk_spark.Network
	switch strings.ToLower(*network) {
	case "regtest":
		networkEnum = breez_sdk_spark.NetworkRegtest
	case "mainnet":
		networkEnum = breez_sdk_spark.NetworkMainnet
	default:
		log.Fatalf("Invalid network. Use 'regtest' or 'mainnet'")
	}

	// Init logging
	breez_sdk_spark.InitLogging(&resolvedDir, nil, nil)

	// Persistence
	persistence := &CliPersistence{dataDir: resolvedDir}

	// Config
	config := breez_sdk_spark.DefaultConfig(networkEnum)
	apiKey := os.Getenv("BREEZ_API_KEY")
	if apiKey != "" {
		config.ApiKey = &apiKey
	}

	// Stable balance config
	if *stableBalanceTokenIdentifier != "" {
		defaultActiveTicker := "USDB"
		sbc := breez_sdk_spark.StableBalanceConfig{
			Tokens: []breez_sdk_spark.StableBalanceToken{
				{Ticker: "USDB", TokenIdentifier: *stableBalanceTokenIdentifier},
			},
			DefaultActiveTicker: &defaultActiveTicker,
		}
		if *stableBalanceThreshold > 0 {
			sbc.ThresholdSats = stableBalanceThreshold
		}
		config.StableBalanceConfig = &sbc
	}

	// Resolve seed: passkey or mnemonic
	var seed breez_sdk_spark.Seed
	if *passkeyProviderStr != "" {
		provider, err := parsePasskeyProvider(*passkeyProviderStr)
		if err != nil {
			log.Fatalf("Invalid passkey provider: %v", err)
		}
		prfProvider, err := buildPrfProvider(provider, resolvedDir)
		if err != nil {
			log.Fatalf("PRF initialization failed: %v", err)
		}
		var wn *string
		if *label != "" {
			wn = label
		}
		var apiKeyPtr *string
		if apiKey != "" {
			apiKeyPtr = &apiKey
		}
		seed, err = resolvePasskeySeed(prfProvider, apiKeyPtr, wn, *listLabels, *storeLabel)
		if err != nil {
			log.Fatalf("Passkey seed resolution failed: %v", err)
		}
	} else {
		mnemonic, err := persistence.GetOrCreateMnemonic()
		if err != nil {
			log.Fatalf("Failed to get/create mnemonic: %v", err)
		}
		seed = breez_sdk_spark.SeedMnemonic{Mnemonic: mnemonic}
	}

	// Build SDK
	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	if *postgresConnectionString != "" {
		pgConfig := breez_sdk_spark.DefaultPostgresStorageConfig(*postgresConnectionString)
		builder.WithPostgresStorage(pgConfig)
	} else {
		builder.WithDefaultStorage(resolvedDir)
	}
	if *accountNumber != "" {
		acctNum, err := strconv.ParseUint(*accountNumber, 10, 32)
		if err != nil {
			log.Fatalf("Invalid account number: %v", err)
		}
		acctNum32 := uint32(acctNum)
		builder.WithKeySet(breez_sdk_spark.KeySetConfig{
			KeySetType:      breez_sdk_spark.KeySetTypeDefault,
			UseAddressIndex: false,
			AccountNumber:   &acctNum32,
		})
	}

	sdk, err := builder.Build()
	if err = liftError(err); err != nil {
		log.Fatalf("Failed to build SDK: %v", err)
	}

	// Event listener
	listener := CliEventListener{}
	sdk.AddEventListener(listener)

	// Token issuer
	tokenIssuer := sdk.GetTokenIssuer()

	// Run REPL
	if err := runRepl(sdk, tokenIssuer, networkEnum, persistence); err != nil {
		log.Fatalf("REPL error: %v", err)
	}

	// Cleanup
	if err = liftError(sdk.Disconnect()); err != nil {
		log.Printf("Warning: disconnect error: %v", err)
	}
	fmt.Println("Goodbye!")
}

func runRepl(sdk *breez_sdk_spark.BreezSdk, tokenIssuer *breez_sdk_spark.TokenIssuer, network breez_sdk_spark.Network, persistence *CliPersistence) error {
	// Build completion list
	allCommands := make([]string, 0)
	allCommands = append(allCommands, CommandNames...)
	allCommands = append(allCommands, IssuerCommandNames...)
	allCommands = append(allCommands, ContactCommandNames...)
	allCommands = append(allCommands, "exit", "quit", "help")

	// Build prefix completer items
	completerItems := make([]readline.PrefixCompleterInterface, len(allCommands))
	for i, cmd := range allCommands {
		completerItems[i] = readline.PcItem(cmd)
	}

	networkLabel := "regtest"
	if network == breez_sdk_spark.NetworkMainnet {
		networkLabel = "mainnet"
	}
	promptStr := fmt.Sprintf("breez-spark-cli [%s]> ", networkLabel)

	rl, err := readline.NewEx(&readline.Config{
		Prompt:          promptStr,
		HistoryFile:     persistence.HistoryFile(),
		AutoComplete:    readline.NewPrefixCompleter(completerItems...),
		InterruptPrompt: "^C",
		EOFPrompt:       "^D",
	})
	if err != nil {
		return fmt.Errorf("failed to create readline: %w", err)
	}
	defer rl.Close()

	registry := BuildCommandRegistry()

	fmt.Println("Breez SDK CLI Interactive Mode")
	fmt.Println("Type 'help' for available commands or 'exit' to quit")

	for {
		rl.SetPrompt(promptStr)
		line, err := rl.Readline()
		if err != nil {
			if err == readline.ErrInterrupt {
				fmt.Println("\nCTRL-C")
				break
			}
			if err == io.EOF {
				fmt.Println("\nCTRL-D")
				break
			}
			fmt.Printf("Error: %v\n", err)
			break
		}

		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}

		if line == "exit" || line == "quit" {
			break
		}

		if line == "help" {
			PrintHelp(registry)
			continue
		}

		args := splitArgs(line)
		cmdName := args[0]
		cmdArgs := args[1:]

		if cmdName == "issuer" {
			DispatchIssuerCommand(cmdArgs, tokenIssuer, rl)
		} else if cmdName == "contacts" {
			DispatchContactCommand(cmdArgs, sdk, rl)
		} else if cmd, ok := registry[cmdName]; ok {
			if err := cmd.Run(sdk, rl, cmdArgs); err != nil {
				fmt.Printf("Error: %v\n", err)
			}
		} else {
			fmt.Printf("Unknown command: %s. Type 'help' for available commands.\n", cmdName)
		}
	}

	return nil
}
