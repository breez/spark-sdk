package main

import (
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"reflect"
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
	"advanced check-recover-funds",
	"advanced check-unilateral-exit",
	"advanced export-unilateral-exit-state",
	"advanced import-unilateral-exit-state",
	"advanced recover-funds",
	"advanced unilateral-exit",
}

// BuildAdvancedRegistry returns a map of advanced subcommand name -> AdvancedCommand.
func BuildAdvancedRegistry() map[string]AdvancedCommand {
	return map[string]AdvancedCommand{
		"check-recover-funds": {
			Name:        "check-recover-funds",
			Description: "Read a recovery written by recover-funds back against the chain",
			Run:         handleCheckRecoverFunds,
		},
		"check-unilateral-exit": {
			Name:        "check-unilateral-exit",
			Description: "Check status of a signed unilateral exit against the chain",
			Run:         handleCheckUnilateralExit,
		},
		"export-unilateral-exit-state": {
			Name:        "export-unilateral-exit-state",
			Description: "Export the wallet's unilateral exit state to a file",
			Run:         handleExportUnilateralExitState,
		},
		"import-unilateral-exit-state": {
			Name:        "import-unilateral-exit-state",
			Description: "Import a previously exported unilateral exit state",
			Run:         handleImportUnilateralExitState,
		},
		"recover-funds": {
			Name:        "recover-funds",
			Description: "Recover the funds that left the balance, or with --all every leaf",
			Run:         handleRecoverFunds,
		},
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

// --- export-unilateral-exit-state ---

func handleExportUnilateralExitState(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("export-unilateral-exit-state", flag.ContinueOnError)
	outputFile := fs.String("output-file", "", "File to write the exit state to")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *outputFile == "" {
		fmt.Println("Usage: advanced export-unilateral-exit-state --output-file <path>")
		return nil
	}

	exported, err := sdk.ExportUnilateralExitState()
	if err = liftError(err); err != nil {
		return err
	}

	if err := os.WriteFile(*outputFile, []byte(exported.ExitState), 0o644); err != nil {
		return err
	}
	fmt.Printf("Wrote %d bytes to %s\n", len(exported.ExitState), *outputFile)
	return nil
}

// --- import-unilateral-exit-state ---

func handleImportUnilateralExitState(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("import-unilateral-exit-state", flag.ContinueOnError)
	inputFile := fs.String("input-file", "", "File the exit state was exported to")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *inputFile == "" {
		fmt.Println("Usage: advanced import-unilateral-exit-state --input-file <path>")
		return nil
	}

	exitStateBytes, err := os.ReadFile(*inputFile)
	if err != nil {
		return err
	}

	imported, err := sdk.ImportUnilateralExitState(breez_sdk_spark.ImportUnilateralExitStateRequest{
		ExitState: string(exitStateBytes),
	})
	if err = liftError(err); err != nil {
		return err
	}

	fmt.Printf("Imported %d leaf(s), skipped %d leaf(s) from a different wallet "+
		"and %d that disagree with what this wallet holds, "+
		"left out the exit data of %d leaf(s)\n",
		imported.ImportedLeaves,
		imported.SkippedForeignLeaves,
		imported.SkippedConflictingLeaves,
		imported.SkippedChains,
	)
	return nil
}

// --- unilateral-exit ---

func handleUnilateralExit(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("unilateral-exit", flag.ContinueOnError)
	feeRate := fs.Uint64("fee-rate", 0, "Target fee rate in sat/vByte")
	fundingKind := fs.String("funding-kind", "p2tr", "Funding UTXO kind: p2wpkh or p2tr")
	destination := fs.String("destination", "", "Destination address for the swept funds")
	var leafIDs stringSliceFlag
	fs.Var(&leafIDs, "leaf", "Leaf id to exit (repeatable). Omit to auto-select every profitable leaf.")
	outputFile := fs.String("output-file", "", "File to write the signed exit to, for check-unilateral-exit to read back")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *feeRate == 0 || *destination == "" {
		fmt.Println("Usage: advanced unilateral-exit --fee-rate <sat/vByte> --destination <address> [--funding-kind p2wpkh|p2tr] [--leaf <id> ...] [--output-file <path>]")
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
		selection = breez_sdk_spark.ExitLeafSelectionAll{}
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
	if *outputFile != "" {
		if err := writeExit(*outputFile, response); err != nil {
			return err
		}
	}
	return nil
}

// --- check-unilateral-exit ---

func handleCheckUnilateralExit(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("check-unilateral-exit", flag.ContinueOnError)
	inputFile := fs.String("input-file", "", "File the exit was written to")
	outputFile := fs.String("output-file", "", "File to write the updated exit to. Defaults to --input-file.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *inputFile == "" {
		fmt.Println("Usage: advanced check-unilateral-exit --input-file <path> [--output-file <path>]")
		return nil
	}

	exit, err := readExit(*inputFile)
	if err != nil {
		return err
	}

	checked, err := sdk.CheckUnilateralExit(breez_sdk_spark.CheckUnilateralExitRequest{
		Exit: exit,
	})
	if err = liftError(err); err != nil {
		return err
	}

	fmt.Printf("Verdict: %s\n", serialize(checked.Verdict))
	if _, ok := checked.Verdict.(breez_sdk_spark.UnilateralExitVerdictRedo); ok {
		fmt.Println("  (this exit cannot be finished, quote and build it again)")
	}

	printExitTransactions(checked.Exit)

	outPath := *inputFile
	if *outputFile != "" {
		outPath = *outputFile
	}
	return writeExit(outPath, checked.Exit)
}

// --- recover-funds ---

func handleRecoverFunds(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("recover-funds", flag.ContinueOnError)
	feeRate := fs.Uint64("fee-rate", 0, "Target fee rate in sat/vByte")
	fundingKind := fs.String("funding-kind", "p2tr", "Funding UTXO kind: p2wpkh or p2tr")
	destination := fs.String("destination", "", "Destination address for the recovered funds")
	all := fs.Bool("all", false, "Recover every leaf worth it, including the ones still in the balance. Only for when the operators are unreachable or refuse to serve the wallet.")
	var leafIDs stringSliceFlag
	fs.Var(&leafIDs, "leaf", "Leaf id to recover (repeatable). Omit to recover the leaves that left the balance.")
	outputFile := fs.String("output-file", "", "File to write the signed recovery to, for check-recover-funds to read back")
	if err := fs.Parse(args); err != nil {
		return err
	}

	feeRateSet := false
	fs.Visit(func(f *flag.Flag) {
		if f.Name == "fee-rate" {
			feeRateSet = true
		}
	})
	if !feeRateSet || *destination == "" {
		fmt.Println("Usage: advanced recover-funds --fee-rate <sat/vByte> --destination <address> [--funding-kind p2wpkh|p2tr] [--all | --leaf <id> ...] [--output-file <path>]")
		return nil
	}
	if *all && len(leafIDs) > 0 {
		fmt.Println("Cannot specify both --all and --leaf")
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

	request := breez_sdk_spark.PrepareRecoverFundsRequest{
		FeeRateSatPerVbyte: *feeRate,
		FundingKind:        &cpfpFundingKind,
		Destination:        *destination,
		Selection:          recoverySelection(*all, leafIDs),
	}
	return recoverFunds(sdk, rl, request, *fundingKind, *outputFile)
}

func recoverFunds(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, request breez_sdk_spark.PrepareRecoverFundsRequest, fundingKind string, outputFile string) error {
	prepared, err := sdk.PrepareRecoverFunds(request)
	if err = liftError(err); err != nil {
		return err
	}
	if len(prepared.Leaves) == 0 {
		fmt.Println("Nothing to recover: each selected leaf is finished, not worth recovering " +
			"at this fee rate, or its funds were not found.")
		return nil
	}
	printQuote(prepared)
	if outputFile == "" {
		fmt.Println("Without --output-file the recovery is only printed: check-recover-funds cannot read it back.")
	}

	var fundingInputs []breez_sdk_spark.CpfpInput
	var signer *breez_sdk_spark.CpfpSigner
	if prepared.Funding != nil {
		utxoLine, err := readlinePrompt(rl, fmt.Sprintf("Funding UTXO(s) of at least %d sats, as txid:vout:value:pubkey "+
			"(space-separated; for P2TR the internal key; blank to skip the unilateral exit): ",
			prepared.Funding.SingleUtxoSats))
		if err != nil {
			return err
		}
		if strings.TrimSpace(utxoLine) == "" {
			var cooperative []string
			for _, leaf := range prepared.Leaves {
				if leaf.Method == breez_sdk_spark.RecoveryMethodCooperative {
					cooperative = append(cooperative, leaf.LeafId)
				}
			}
			if len(cooperative) == 0 {
				fmt.Println("Nothing to recover without funding.")
				return nil
			}
			fmt.Println("Recovering only the cooperative leaves:")
			request.Selection = breez_sdk_spark.ExitLeafSelectionSpecific{LeafIds: cooperative}
			prepared, err = sdk.PrepareRecoverFunds(request)
			if err = liftError(err); err != nil {
				return err
			}
			printQuote(prepared)
		} else {
			for _, u := range strings.Fields(utxoLine) {
				input, err := parseCpfpInput(u, fundingKind)
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
			cpfpSigner, err := breez_sdk_spark.SingleKeyCpfpSigner(secretKeyBytes)
			if err = liftError(err); err != nil {
				return err
			}
			signer = &cpfpSigner
		}
	}

	answer, err := readlineWithDefault(rl, "Sign this recovery? (y/n): ", "y")
	if err != nil {
		return err
	}
	if strings.ToLower(strings.TrimSpace(answer)) != "y" {
		return nil
	}
	response, err := sdk.RecoverFunds(breez_sdk_spark.RecoverFundsRequest{
		Prepared:      prepared,
		FundingInputs: fundingInputs,
	}, signer)
	if err = liftError(err); err != nil {
		return err
	}
	printRecovery(response)
	if outputFile == "" {
		fmt.Println("Next: broadcast the Ready packages.")
		return nil
	}
	if err := writeRecovery(outputFile, response); err != nil {
		return err
	}
	fmt.Printf("Next: broadcast the Ready packages. After new blocks, run check-recover-funds "+
		"--input-file %s to see what is ready next.\n", outputFile)
	return nil
}

func printQuote(prepared breez_sdk_spark.PrepareRecoverFundsResponse) {
	printValue(prepared)
	cooperative := 0
	for _, leaf := range prepared.Leaves {
		if leaf.Method == breez_sdk_spark.RecoveryMethodCooperative {
			cooperative++
		}
	}
	fmt.Printf("%d leaf(s), %d cooperative and %d unilateral: recovering %d sats for %d sats in fees\n",
		len(prepared.Leaves), cooperative, len(prepared.Leaves)-cooperative,
		prepared.RecoverableValueSats, prepared.TotalFeeSats)
}

func recoverySelection(all bool, leafIDs []string) breez_sdk_spark.ExitLeafSelection {
	if all {
		return breez_sdk_spark.ExitLeafSelectionAll{}
	}
	if len(leafIDs) == 0 {
		return breez_sdk_spark.ExitLeafSelectionRecoverableOnly{}
	}
	return breez_sdk_spark.ExitLeafSelectionSpecific{LeafIds: leafIDs}
}

// --- check-recover-funds ---

func handleCheckRecoverFunds(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("check-recover-funds", flag.ContinueOnError)
	inputFile := fs.String("input-file", "", "File the recovery was written to")
	outputFile := fs.String("output-file", "", "File to write the updated recovery to. Defaults to --input-file.")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *inputFile == "" {
		fmt.Println("Usage: advanced check-recover-funds --input-file <path> [--output-file <path>]")
		return nil
	}

	return checkRecoverFunds(sdk, *inputFile, *outputFile)
}

func checkRecoverFunds(sdk *breez_sdk_spark.BreezSdk, inputFile string, outputFile string) error {
	recovery, err := readRecovery(inputFile)
	if err != nil {
		return err
	}

	checked, err := sdk.CheckRecoverFunds(breez_sdk_spark.CheckRecoverFundsRequest{
		Recovery: recovery,
	})
	if err = liftError(err); err != nil {
		return err
	}

	fmt.Printf("Verdict: %s\n", serialize(checked.Verdict))
	if _, ok := checked.Verdict.(breez_sdk_spark.RecoveryVerdictRedo); ok {
		fmt.Printf("  (this recovery cannot finish: run %s)\n", redoCommand(checked.Recovery))
	}

	printRecovery(checked.Recovery)

	outPath := inputFile
	if outputFile != "" {
		outPath = outputFile
	}
	return writeRecovery(outPath, checked.Recovery)
}

func redoCommand(recovery breez_sdk_spark.RecoverFundsResponse) string {
	command := fmt.Sprintf("recover-funds --fee-rate %d --destination %s",
		recovery.FeeRateSatPerVbyte, recovery.Destination)
	for _, leaf := range recovery.Leaves {
		command += " --leaf " + leaf.LeafId
	}
	return command
}

// ---------------------------------------------------------------------------
// Exit file I/O
// ---------------------------------------------------------------------------

func writeExit(path string, response breez_sdk_spark.UnilateralExitResponse) error {
	data, err := json.MarshalIndent(objToMap(response), "", "  ")
	if err != nil {
		return err
	}
	if err := os.WriteFile(path, append(data, '\n'), 0o644); err != nil {
		return err
	}
	fmt.Printf("Wrote the exit to %s\n", path)
	return nil
}

func readExit(path string) (breez_sdk_spark.UnilateralExitResponse, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return breez_sdk_spark.UnilateralExitResponse{}, err
	}
	var raw map[string]interface{}
	if err := json.Unmarshal(data, &raw); err != nil {
		return breez_sdk_spark.UnilateralExitResponse{}, err
	}
	return exitResponseFromMap(raw), nil
}

func exitResponseFromMap(m map[string]interface{}) breez_sdk_spark.UnilateralExitResponse {
	resp := breez_sdk_spark.UnilateralExitResponse{
		RecoverableValueSat: mapUint64(m, "recoverable_value_sat"),
		TotalFeeSat:         mapUint64(m, "total_fee_sat"),
		CpfpFeeSat:          mapUint64(m, "cpfp_fee_sat"),
		FanoutFeeSat:        mapUint64(m, "fanout_fee_sat"),
		SweepFeeSat:         mapUint64(m, "sweep_fee_sat"),
	}

	if leaves, ok := m["leaves"].([]interface{}); ok {
		for _, l := range leaves {
			if lm, ok := l.(map[string]interface{}); ok {
				resp.Leaves = append(resp.Leaves, breez_sdk_spark.UnilateralExitLeaf{
					LeafId: mapStr(lm, "leaf_id"),
					Value:  mapUint64(lm, "value"),
				})
			}
		}
	}

	if txs, ok := m["transactions"].([]interface{}); ok {
		for _, t := range txs {
			if tm, ok := t.(map[string]interface{}); ok {
				resp.Transactions = append(resp.Transactions, breez_sdk_spark.UnilateralExitTransaction{
					Kind:              exitTxKindFromMap(tm["kind"]),
					NodeId:            mapOptStr(tm, "node_id"),
					Txid:              mapStr(tm, "txid"),
					TxHex:             mapStr(tm, "tx_hex"),
					CpfpTxHex:         mapOptStr(tm, "cpfp_tx_hex"),
					CsvTimelockBlocks: mapOptUint32(tm, "csv_timelock_blocks"),
					DependsOn:         mapStrSlice(tm, "depends_on"),
					Status:            exitTxStatusFromMap(tm["status"]),
				})
			}
		}
	}

	if inputs, ok := m["funding_inputs"].([]interface{}); ok {
		for _, fi := range inputs {
			if im, ok := fi.(map[string]interface{}); ok {
				resp.FundingInputs = append(resp.FundingInputs, cpfpInputFromMap(im))
			}
		}
	}

	return resp
}

func exitTxKindFromMap(v interface{}) breez_sdk_spark.UnilateralExitTxKind {
	f, ok := v.(float64)
	if !ok {
		return breez_sdk_spark.UnilateralExitTxKindFanOut
	}
	return breez_sdk_spark.UnilateralExitTxKind(uint(f))
}

func exitTxStatusFromMap(v interface{}) breez_sdk_spark.ExitTransactionStatus {
	m, ok := v.(map[string]interface{})
	if !ok {
		return breez_sdk_spark.ExitTransactionStatusUnverified{}
	}
	switch mapStr(m, "type") {
	case "Confirmed":
		return breez_sdk_spark.ExitTransactionStatusConfirmed{
			BlockHeight: mapOptUint32(m, "block_height"),
		}
	case "WaitingForDependencies":
		return breez_sdk_spark.ExitTransactionStatusWaitingForDependencies{}
	case "WaitingForTimelock":
		return breez_sdk_spark.ExitTransactionStatusWaitingForTimelock{
			SpendableAtHeight: mapOptUint32(m, "spendable_at_height"),
		}
	case "Ready":
		return breez_sdk_spark.ExitTransactionStatusReady{}
	default:
		return breez_sdk_spark.ExitTransactionStatusUnverified{}
	}
}

func cpfpInputFromMap(m map[string]interface{}) breez_sdk_spark.CpfpInput {
	switch mapStr(m, "type") {
	case "P2wpkh":
		return breez_sdk_spark.CpfpInputP2wpkh{
			Txid:      mapStr(m, "txid"),
			Vout:      uint32(mapUint64(m, "vout")),
			ValueSats: mapUint64(m, "value_sats"),
			Pubkey:    mapStr(m, "pubkey"),
		}
	case "Custom":
		return breez_sdk_spark.CpfpInputCustom{
			Txid:              mapStr(m, "txid"),
			Vout:              uint32(mapUint64(m, "vout")),
			ValueSats:         mapUint64(m, "value_sats"),
			ScriptPubkeyHex:   mapStr(m, "script_pubkey_hex"),
			SignedInputWeight: mapUint64(m, "signed_input_weight"),
		}
	default:
		return breez_sdk_spark.CpfpInputP2tr{
			Txid:      mapStr(m, "txid"),
			Vout:      uint32(mapUint64(m, "vout")),
			ValueSats: mapUint64(m, "value_sats"),
			Pubkey:    mapStr(m, "pubkey"),
		}
	}
}

// JSON map accessors for exit file deserialization.

func mapUint64(m map[string]interface{}, key string) uint64 {
	v, _ := m[key].(float64)
	return uint64(v)
}

func mapStr(m map[string]interface{}, key string) string {
	v, _ := m[key].(string)
	return v
}

func mapOptStr(m map[string]interface{}, key string) *string {
	v, ok := m[key]
	if !ok || v == nil {
		return nil
	}
	s, ok := v.(string)
	if !ok {
		return nil
	}
	return &s
}

func mapOptUint64(m map[string]interface{}, key string) *uint64 {
	v, ok := m[key]
	if !ok || v == nil {
		return nil
	}
	f, ok := v.(float64)
	if !ok {
		return nil
	}
	u := uint64(f)
	return &u
}

func mapOptUint32(m map[string]interface{}, key string) *uint32 {
	v, ok := m[key]
	if !ok || v == nil {
		return nil
	}
	f, ok := v.(float64)
	if !ok {
		return nil
	}
	u := uint32(f)
	return &u
}

func mapStrSlice(m map[string]interface{}, key string) []string {
	v, ok := m[key]
	if !ok || v == nil {
		return nil
	}
	arr, ok := v.([]interface{})
	if !ok {
		return nil
	}
	result := make([]string, 0, len(arr))
	for _, item := range arr {
		if s, ok := item.(string); ok {
			result = append(result, s)
		}
	}
	return result
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
			Txid:      txid,
			Vout:      uint32(vout),
			ValueSats: value,
			Pubkey:    pubkey,
		}, nil
	default:
		return breez_sdk_spark.CpfpInputP2tr{
			Txid:      txid,
			Vout:      uint32(vout),
			ValueSats: value,
			Pubkey:    pubkey,
		}, nil
	}
}

func printExitTransactions(response breez_sdk_spark.UnilateralExitResponse) {
	fmt.Printf("Recoverable %d sats, total fee %d sats (cpfp %d, fanout %d, sweep %d), %d transaction(s):\n",
		response.RecoverableValueSat, response.TotalFeeSat,
		response.CpfpFeeSat, response.FanoutFeeSat, response.SweepFeeSat,
		len(response.Transactions))
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
		switch s := tx.Status.(type) {
		case breez_sdk_spark.ExitTransactionStatusConfirmed:
			if s.BlockHeight != nil {
				fmt.Printf("      (confirmed in block %d, nothing to broadcast)\n", *s.BlockHeight)
			} else {
				fmt.Println("      (already confirmed, nothing to broadcast)")
			}
			continue
		case breez_sdk_spark.ExitTransactionStatusWaitingForDependencies:
			fmt.Println("      (waiting on the transactions it depends on)")
		case breez_sdk_spark.ExitTransactionStatusWaitingForTimelock:
			if s.SpendableAtHeight != nil {
				fmt.Printf("      (waiting for its timelock, until block %d)\n", *s.SpendableAtHeight)
			} else {
				fmt.Println("      (waiting for its timelock)")
			}
		case breez_sdk_spark.ExitTransactionStatusReady:
		case breez_sdk_spark.ExitTransactionStatusUnverified:
		default:
		}
		pkg := tx.TxHex
		if tx.CpfpTxHex != nil {
			pkg = tx.TxHex + "," + *tx.CpfpTxHex
		}
		fmt.Printf("      Package: %s\n", pkg)
	}
}

// ---------------------------------------------------------------------------
// Recovery file I/O
// ---------------------------------------------------------------------------

func writeRecovery(path string, recovery breez_sdk_spark.RecoverFundsResponse) error {
	data, err := json.MarshalIndent(objToMap(recovery), "", "  ")
	if err != nil {
		return err
	}
	temporary := path + ".tmp"
	if err := os.WriteFile(temporary, append(data, '\n'), 0o644); err != nil {
		return err
	}
	if err := os.Rename(temporary, path); err != nil {
		return err
	}
	fmt.Printf("Wrote the recovery to %s\n", path)
	return nil
}

func readRecovery(path string) (breez_sdk_spark.RecoverFundsResponse, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return breez_sdk_spark.RecoverFundsResponse{}, err
	}
	var raw map[string]interface{}
	if err := json.Unmarshal(data, &raw); err != nil {
		return breez_sdk_spark.RecoverFundsResponse{}, err
	}
	return recoveryFromMap(raw), nil
}

func recoveryFromMap(m map[string]interface{}) breez_sdk_spark.RecoverFundsResponse {
	resp := breez_sdk_spark.RecoverFundsResponse{
		RecoverableValueSats: mapUint64(m, "recoverable_value_sats"),
		TotalFeeSats:         mapUint64(m, "total_fee_sats"),
		CooperativeFeeSats:   mapUint64(m, "cooperative_fee_sats"),
		CpfpFeeSats:          mapUint64(m, "cpfp_fee_sats"),
		FanoutFeeSats:        mapUint64(m, "fanout_fee_sats"),
		SweepFeeSats:         mapUint64(m, "sweep_fee_sats"),
		FeeRateSatPerVbyte:   mapUint64(m, "fee_rate_sat_per_vbyte"),
		Destination:          mapStr(m, "destination"),
	}

	if leaves, ok := m["leaves"].([]interface{}); ok {
		for _, l := range leaves {
			if lm, ok := l.(map[string]interface{}); ok {
				resp.Leaves = append(resp.Leaves, breez_sdk_spark.RecoverFundsLeaf{
					LeafId:    mapStr(lm, "leaf_id"),
					ValueSats: mapUint64(lm, "value_sats"),
					Method:    breez_sdk_spark.RecoveryMethod(mapUint64(lm, "method")),
				})
			}
		}
	}

	if failed, ok := m["failed"].([]interface{}); ok {
		for _, f := range failed {
			if fm, ok := f.(map[string]interface{}); ok {
				resp.Failed = append(resp.Failed, breez_sdk_spark.CooperativeRecoveryFailure{
					LeafId:     mapStr(fm, "leaf_id"),
					OutputTxid: mapStr(fm, "output_txid"),
					OutputVout: uint32(mapUint64(fm, "output_vout")),
					Error:      cooperativeRecoveryErrorFromMap(fm["error"]),
				})
			}
		}
	}

	if txs, ok := m["transactions"].([]interface{}); ok {
		for _, t := range txs {
			if tm, ok := t.(map[string]interface{}); ok {
				resp.Transactions = append(resp.Transactions, breez_sdk_spark.RecoveryTransaction{
					Kind:              breez_sdk_spark.RecoveryTxKind(mapUint64(tm, "kind")),
					NodeId:            mapOptStr(tm, "node_id"),
					Txid:              mapStr(tm, "txid"),
					TxHex:             mapStr(tm, "tx_hex"),
					CpfpTxHex:         mapOptStr(tm, "cpfp_tx_hex"),
					CsvTimelockBlocks: mapOptUint32(tm, "csv_timelock_blocks"),
					DependsOn:         mapStrSlice(tm, "depends_on"),
					Status:            exitTxStatusFromMap(tm["status"]),
				})
			}
		}
	}

	if inputs, ok := m["funding_inputs"].([]interface{}); ok {
		for _, fi := range inputs {
			if im, ok := fi.(map[string]interface{}); ok {
				resp.FundingInputs = append(resp.FundingInputs, cpfpInputFromMap(im))
			}
		}
	}

	return resp
}

func cooperativeRecoveryErrorFromMap(v interface{}) breez_sdk_spark.CooperativeRecoveryError {
	m, _ := v.(map[string]interface{})
	switch mapStr(m, "type") {
	case "ReplacementFeeTooLow":
		return breez_sdk_spark.CooperativeRecoveryErrorReplacementFeeTooLow{
			RequiredFeeSats:            mapUint64(m, "required_fee_sats"),
			RequiredFeeRateSatPerVbyte: mapUint64(m, "required_fee_rate_sat_per_vbyte"),
		}
	case "OperatorsUnavailable":
		return breez_sdk_spark.CooperativeRecoveryErrorOperatorsUnavailable{
			Message: mapStr(m, "message"),
		}
	default:
		return breez_sdk_spark.CooperativeRecoveryErrorGeneric{
			Message: mapStr(m, "message"),
		}
	}
}

func printRecovery(response breez_sdk_spark.RecoverFundsResponse) {
	fmt.Printf("Recoverable %d sats, total fee %d sats (cooperative %d, cpfp %d, fanout %d, sweep %d), %d transaction(s):\n",
		response.RecoverableValueSats, response.TotalFeeSats, response.CooperativeFeeSats,
		response.CpfpFeeSats, response.FanoutFeeSats, response.SweepFeeSats,
		len(response.Transactions))
	for i, tx := range response.Transactions {
		after := ""
		if len(tx.DependsOn) > 0 {
			after = ", after " + strings.Join(tx.DependsOn, ",")
		}
		csv := ""
		if tx.CsvTimelockBlocks != nil {
			csv = fmt.Sprintf(", csv %d blocks", *tx.CsvTimelockBlocks)
		}
		node := ""
		if tx.NodeId != nil {
			node = " node=" + *tx.NodeId
		}
		fmt.Printf("  [%d] %s%s status=%s txid=%s%s%s\n",
			i, recoveryTxKindName(tx.Kind), node,
			extractVariantName(reflect.TypeOf(tx.Status).Name()), tx.Txid, after, csv)
		switch s := tx.Status.(type) {
		case breez_sdk_spark.ExitTransactionStatusConfirmed:
			if s.BlockHeight != nil {
				fmt.Printf("      (confirmed in block %d, nothing to broadcast)\n", *s.BlockHeight)
			} else {
				fmt.Println("      (already confirmed, nothing to broadcast)")
			}
			continue
		case breez_sdk_spark.ExitTransactionStatusWaitingForDependencies:
			fmt.Println("      (waiting on the transactions it depends on)")
		case breez_sdk_spark.ExitTransactionStatusWaitingForTimelock:
			if s.SpendableAtHeight != nil {
				fmt.Printf("      (waiting for its timelock, until block %d)\n", *s.SpendableAtHeight)
			} else {
				fmt.Println("      (waiting for its timelock)")
			}
		}
		pkg := tx.TxHex
		if tx.CpfpTxHex != nil {
			pkg = tx.TxHex + "," + *tx.CpfpTxHex
		}
		fmt.Printf("      Package: %s\n", pkg)
	}
	if len(response.Failed) > 0 {
		fmt.Printf("Not recovered, %d leaf(s):\n", len(response.Failed))
	}
	for _, failure := range response.Failed {
		fmt.Printf("  leaf %s (output %s:%d): %s\n",
			failure.LeafId, failure.OutputTxid, failure.OutputVout,
			cooperativeRecoveryErrorMessage(failure.Error))
	}
}

func recoveryTxKindName(kind breez_sdk_spark.RecoveryTxKind) string {
	switch kind {
	case breez_sdk_spark.RecoveryTxKindCooperative:
		return "Cooperative"
	case breez_sdk_spark.RecoveryTxKindFanOut:
		return "FanOut"
	case breez_sdk_spark.RecoveryTxKindNode:
		return "Node"
	case breez_sdk_spark.RecoveryTxKindRefund:
		return "Refund"
	case breez_sdk_spark.RecoveryTxKindSweep:
		return "Sweep"
	}
	return fmt.Sprintf("%d", kind)
}

func cooperativeRecoveryErrorMessage(e breez_sdk_spark.CooperativeRecoveryError) string {
	switch e := e.(type) {
	case breez_sdk_spark.CooperativeRecoveryErrorReplacementFeeTooLow:
		return fmt.Sprintf("A recovery of this output is already on the network: replacing it takes at least %d sats or %d sats/vbyte",
			e.RequiredFeeSats, e.RequiredFeeRateSatPerVbyte)
	case breez_sdk_spark.CooperativeRecoveryErrorOperatorsUnavailable:
		return fmt.Sprintf("Operators unavailable: %s", e.Message)
	case breez_sdk_spark.CooperativeRecoveryErrorGeneric:
		return fmt.Sprintf("Generic error: %s", e.Message)
	}
	return fmt.Sprintf("%v", e)
}
