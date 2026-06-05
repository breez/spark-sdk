package main

import (
	"fmt"
	"sort"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// StableBalanceCmd represents a single stable-balance subcommand.
type StableBalanceCmd struct {
	Name        string
	Description string
	Run         func(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error
}

// StableBalanceCommandNames lists all stable-balance subcommand names (used for REPL completion).
var StableBalanceCommandNames = []string{
	"stable-balance get",
	"stable-balance set",
	"stable-balance unset",
}

// BuildStableBalanceRegistry returns a map of stable-balance subcommand name to StableBalanceCmd.
func BuildStableBalanceRegistry() map[string]StableBalanceCmd {
	return map[string]StableBalanceCmd{
		"get": {
			Name:        "get",
			Description: "Get the stable balance active label",
			Run:         handleStableBalanceGet,
		},
		"set": {
			Name:        "set",
			Description: "Set the stable balance active label",
			Run:         handleStableBalanceSet,
		},
		"unset": {
			Name:        "unset",
			Description: "Unset stable balance",
			Run:         handleStableBalanceUnset,
		},
	}
}

// DispatchStableBalanceCommand dispatches a stable-balance subcommand.
func DispatchStableBalanceCommand(args []string, sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance) {
	registry := BuildStableBalanceRegistry()

	if len(args) == 0 || args[0] == "help" {
		fmt.Println("\nStable balance subcommands:")
		names := make([]string, 0, len(registry))
		for name := range registry {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			cmd := registry[name]
			fmt.Printf("  stable-balance %-24s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	subName := args[0]
	subArgs := args[1:]

	cmd, ok := registry[subName]
	if !ok {
		fmt.Printf("Unknown stable-balance subcommand: %s. Use 'stable-balance help' for available commands.\n", subName)
		return
	}

	if err := cmd.Run(sdk, rl, subArgs); err != nil {
		fmt.Printf("Error: %v\n", err)
	}
}

// ---------------------------------------------------------------------------
// Stable balance command handlers
// ---------------------------------------------------------------------------

// --- get ---

func handleStableBalanceGet(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.GetUserSettings()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result.StableBalanceActiveLabel)
	return nil
}

// --- set ---

func handleStableBalanceSet(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: stable-balance set <label>")
		return nil
	}

	var activeLabel breez_sdk_spark.StableBalanceActiveLabel = breez_sdk_spark.StableBalanceActiveLabelSet{
		Label: args[0],
	}
	if err := liftError(sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
		StableBalanceActiveLabel: &activeLabel,
	})); err != nil {
		return err
	}
	result, err := sdk.GetUserSettings()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- unset ---

func handleStableBalanceUnset(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	var activeLabel breez_sdk_spark.StableBalanceActiveLabel = breez_sdk_spark.StableBalanceActiveLabelUnset{}
	if err := liftError(sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
		StableBalanceActiveLabel: &activeLabel,
	})); err != nil {
		return err
	}
	result, err := sdk.GetUserSettings()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}
