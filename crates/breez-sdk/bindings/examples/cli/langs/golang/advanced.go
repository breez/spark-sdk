package main

import (
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"os"
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
	"advanced check-unilateral-exit",
	"advanced export-unilateral-exit-state",
	"advanced import-unilateral-exit-state",
	"advanced unilateral-exit",
}

// BuildAdvancedRegistry returns a map of advanced subcommand name -> AdvancedCommand.
func BuildAdvancedRegistry() map[string]AdvancedCommand {
	return map[string]AdvancedCommand{
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
			Txid:   mapStr(m, "txid"),
			Vout:   uint32(mapUint64(m, "vout")),
			Value:  mapUint64(m, "value"),
			Pubkey: mapStr(m, "pubkey"),
		}
	case "Custom":
		return breez_sdk_spark.CpfpInputCustom{
			Txid:              mapStr(m, "txid"),
			Vout:              uint32(mapUint64(m, "vout")),
			Value:             mapUint64(m, "value"),
			ScriptPubkeyHex:   mapStr(m, "script_pubkey_hex"),
			SignedInputWeight: mapUint64(m, "signed_input_weight"),
		}
	default:
		return breez_sdk_spark.CpfpInputP2tr{
			Txid:   mapStr(m, "txid"),
			Vout:   uint32(mapUint64(m, "vout")),
			Value:  mapUint64(m, "value"),
			Pubkey: mapStr(m, "pubkey"),
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
