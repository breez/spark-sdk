package main

import (
	"encoding/hex"
	"flag"
	"fmt"
	"sort"
	"strconv"
	"strings"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// AdvancedCommand represents a single advanced subcommand.
type AdvancedCommand struct {
	Name        string
	Description string
	Run         func(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error
}

// AdvancedCommandNames lists all advanced subcommand names (used for REPL completion).
var AdvancedCommandNames = []string{
	"advanced unilateral-exit",
}

// BuildAdvancedRegistry returns a map of advanced subcommand name -> AdvancedCommand.
func BuildAdvancedRegistry() map[string]AdvancedCommand {
	return map[string]AdvancedCommand{
		"unilateral-exit": {
			Name:        "unilateral-exit",
			Description: "Build and sign a unilateral exit",
			Run:         handleUnilateralExit,
		},
	}
}

// DispatchAdvancedCommand dispatches an advanced subcommand.
func DispatchAdvancedCommand(args []string, sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance) {
	registry := BuildAdvancedRegistry()

	if len(args) == 0 || args[0] == "help" {
		fmt.Println("\nAdvanced subcommands (expert-only, misuse can strand or lose funds):")
		names := make([]string, 0, len(registry))
		for name := range registry {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			cmd := registry[name]
			fmt.Printf("  advanced %-30s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	subName := args[0]
	subArgs := args[1:]

	cmd, ok := registry[subName]
	if !ok {
		fmt.Printf("Unknown advanced subcommand: %s. Use 'advanced help' for available commands.\n", subName)
		return
	}

	if err := cmd.Run(sdk, rl, subArgs); err != nil {
		fmt.Printf("Error: %v\n", err)
	}
}

// --- unilateral-exit ---

func handleUnilateralExit(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("unilateral-exit", flag.ContinueOnError)
	feeRate := fs.Uint64("fee-rate", 0, "Target fee rate in sat/vByte")
	fundingKind := fs.String("funding-kind", "p2tr", "Funding UTXO kind: p2wpkh or p2tr")
	destination := fs.String("destination", "", "Destination address for the swept funds")
	var leafIDs stringSliceFlag
	fs.Var(&leafIDs, "leaf", "Leaf id to exit (repeatable). Omit to auto-select every profitable leaf.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *feeRate == 0 || *destination == "" {
		fmt.Println("Usage: advanced unilateral-exit --fee-rate <sat/vByte> --destination <address> [--funding-kind p2wpkh|p2tr] [--leaf <id> ...]")
		return nil
	}

	var cpfpFundingKind breez_sdk_spark.CpfpFundingKind
	switch strings.ToLower(*fundingKind) {
	case "p2wpkh":
		cpfpFundingKind = breez_sdk_spark.CpfpFundingKindP2wpkh{}
	case "p2tr":
		cpfpFundingKind = breez_sdk_spark.CpfpFundingKindP2tr{}
	default:
		return fmt.Errorf("invalid funding kind '%s', expected p2wpkh or p2tr", *fundingKind)
	}

	var selection breez_sdk_spark.ExitLeafSelection
	if len(leafIDs) == 0 {
		selection = breez_sdk_spark.ExitLeafSelectionAuto{}
	} else {
		selection = breez_sdk_spark.ExitLeafSelectionSpecific{LeafIds: leafIDs}
	}

	prepared, err := sdk.PrepareUnilateralExit(breez_sdk_spark.PrepareUnilateralExitRequest{
		FeeRateSatPerVbyte: *feeRate,
		FundingKind:        cpfpFundingKind,
		Destination:        *destination,
		Selection:          selection,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(prepared)

	if len(prepared.Leaves) == 0 {
		fmt.Println("No leaves to exit.")
		return nil
	}

	utxoLine, err := readlinePrompt(rl, "Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): ")
	if err != nil {
		return err
	}
	if strings.TrimSpace(utxoLine) == "" {
		fmt.Println("No funding provided; showing the quote only.")
		return nil
	}

	var fundingInputs []breez_sdk_spark.CpfpInput
	for _, u := range strings.Fields(utxoLine) {
		input, err := parseCpfpInput(u, *fundingKind)
		if err != nil {
			return err
		}
		fundingInputs = append(fundingInputs, input)
	}

	keyLine, err := readlinePrompt(rl, "Hex secret key for the funding UTXO(s): ")
	if err != nil {
		return err
	}
	secretKeyBytes, err := hex.DecodeString(strings.TrimSpace(keyLine))
	if err != nil {
		return fmt.Errorf("invalid hex key: %w", err)
	}
	signer, err := breez_sdk_spark.SingleKeyCpfpSigner(secretKeyBytes)
	if err = liftError(err); err != nil {
		return err
	}

	response, err := sdk.UnilateralExit(breez_sdk_spark.UnilateralExitRequest{
		Prepared:      prepared,
		FundingInputs: fundingInputs,
	}, signer)
	if err = liftError(err); err != nil {
		return err
	}
	printExitTransactions(response)
	return nil
}

func parseCpfpInput(s string, kindStr string) (breez_sdk_spark.CpfpInput, error) {
	parts := strings.Split(s, ":")
	if len(parts) != 4 {
		return nil, fmt.Errorf("invalid funding UTXO '%s', expected txid:vout:value:pubkey", s)
	}
	txid := parts[0]
	vout, err := strconv.ParseUint(parts[1], 10, 32)
	if err != nil {
		return nil, fmt.Errorf("invalid vout in '%s': %w", s, err)
	}
	value, err := strconv.ParseUint(parts[2], 10, 64)
	if err != nil {
		return nil, fmt.Errorf("invalid value in '%s': %w", s, err)
	}
	pubkey := parts[3]

	switch strings.ToLower(kindStr) {
	case "p2wpkh":
		return breez_sdk_spark.CpfpInputP2wpkh{
			Txid:   txid,
			Vout:   uint32(vout),
			Value:  value,
			Pubkey: pubkey,
		}, nil
	default:
		return breez_sdk_spark.CpfpInputP2tr{
			Txid:   txid,
			Vout:   uint32(vout),
			Value:  value,
			Pubkey: pubkey,
		}, nil
	}
}

func printExitTransactions(response breez_sdk_spark.UnilateralExitResponse) {
	fmt.Printf("Recoverable %d sats, total fee %d sats, %d transaction(s):\n",
		response.RecoverableValueSat, response.TotalFeeSat, len(response.Transactions))
	for i, tx := range response.Transactions {
		after := ""
		if len(tx.DependsOn) > 0 {
			after = ", after " + strings.Join(tx.DependsOn, ",")
		}
		csv := ""
		if tx.CsvTimelockBlocks != nil {
			csv = fmt.Sprintf(", csv %d blocks", *tx.CsvTimelockBlocks)
		}
		fmt.Printf("  [%d] %v status=%v txid=%s%s%s\n",
			i, tx.Kind, tx.Status, tx.Txid, after, csv)
		if tx.Status == breez_sdk_spark.ConfirmationStatusConfirmed {
			fmt.Println("      (already confirmed, nothing to broadcast)")
			continue
		}
		pkg := tx.TxHex
		if tx.CpfpTxHex != nil {
			pkg = tx.TxHex + "," + *tx.CpfpTxHex
		}
		fmt.Printf("      Package: %s\n", pkg)
	}
}
