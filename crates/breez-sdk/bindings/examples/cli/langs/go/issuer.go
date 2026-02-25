package main

import (
	"flag"
	"fmt"
	"math/big"
	"sort"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// IssuerCommand represents a single issuer subcommand.
type IssuerCommand struct {
	Name        string
	Description string
	Run         func(issuer *breez_sdk_spark.TokenIssuer, rl *readline.Instance, args []string) error
}

// IssuerCommandNames lists all issuer subcommand names (used for REPL completion).
var IssuerCommandNames = []string{
	"issuer token-balance",
	"issuer token-metadata",
	"issuer create-token",
	"issuer mint-token",
	"issuer burn-token",
	"issuer freeze-token",
	"issuer unfreeze-token",
}

// BuildIssuerRegistry returns a map of issuer subcommand name → IssuerCommand.
func BuildIssuerRegistry() map[string]IssuerCommand {
	return map[string]IssuerCommand{
		"token-balance": {
			Name:        "token-balance",
			Description: "Get issuer token balance",
			Run:         handleTokenBalance,
		},
		"token-metadata": {
			Name:        "token-metadata",
			Description: "Get issuer token metadata",
			Run:         handleTokenMetadata,
		},
		"create-token": {
			Name:        "create-token",
			Description: "Create a new issuer token",
			Run:         handleCreateToken,
		},
		"mint-token": {
			Name:        "mint-token",
			Description: "Mint supply of the issuer token",
			Run:         handleMintToken,
		},
		"burn-token": {
			Name:        "burn-token",
			Description: "Burn supply of the issuer token",
			Run:         handleBurnToken,
		},
		"freeze-token": {
			Name:        "freeze-token",
			Description: "Freeze tokens at an address",
			Run:         handleFreezeToken,
		},
		"unfreeze-token": {
			Name:        "unfreeze-token",
			Description: "Unfreeze tokens at an address",
			Run:         handleUnfreezeToken,
		},
	}
}

// DispatchIssuerCommand dispatches an issuer subcommand.
func DispatchIssuerCommand(args []string, issuer *breez_sdk_spark.TokenIssuer, rl *readline.Instance) {
	registry := BuildIssuerRegistry()

	if len(args) == 0 || args[0] == "help" {
		fmt.Println("\nIssuer subcommands:\n")
		names := make([]string, 0, len(registry))
		for name := range registry {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			cmd := registry[name]
			fmt.Printf("  issuer %-30s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	subName := args[0]
	subArgs := args[1:]

	cmd, ok := registry[subName]
	if !ok {
		fmt.Printf("Unknown issuer subcommand: %s. Use 'issuer help' for available commands.\n", subName)
		return
	}

	if err := cmd.Run(issuer, rl, subArgs); err != nil {
		fmt.Printf("Error: %v\n", err)
	}
}

// ---------------------------------------------------------------------------
// Issuer command handlers
// ---------------------------------------------------------------------------

// --- token-balance ---

func handleTokenBalance(issuer *breez_sdk_spark.TokenIssuer, _ *readline.Instance, _ []string) error {
	result, err := issuer.GetIssuerTokenBalance()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- token-metadata ---

func handleTokenMetadata(issuer *breez_sdk_spark.TokenIssuer, _ *readline.Instance, _ []string) error {
	result, err := issuer.GetIssuerTokenMetadata()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- create-token ---

func handleCreateToken(issuer *breez_sdk_spark.TokenIssuer, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("create-token", flag.ContinueOnError)
	name := fs.String("name", "", "Token name (required)")
	ticker := fs.String("ticker", "", "Token ticker (required)")
	decimals := fs.Uint("decimals", 6, "Token decimals")
	freezable := fs.Bool("freezable", false, "Whether the token can be frozen")
	maxSupplyStr := fs.String("max-supply", "", "Maximum supply (optional)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *name == "" || *ticker == "" {
		fmt.Println("Usage: issuer create-token --name <name> --ticker <ticker> [--decimals N] [--freezable] [--max-supply N]")
		return nil
	}

	decimalsU32 := uint32(*decimals)
	req := breez_sdk_spark.CreateIssuerTokenRequest{
		Name:        *name,
		Ticker:      *ticker,
		Decimals:    decimalsU32,
		IsFreezable: *freezable,
	}

	if *maxSupplyStr != "" {
		maxSupply, ok := new(big.Int).SetString(*maxSupplyStr, 10)
		if !ok {
			return fmt.Errorf("invalid max-supply: %s", *maxSupplyStr)
		}
		req.MaxSupply = maxSupply
	}

	result, err := issuer.CreateIssuerToken(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- mint-token ---

func handleMintToken(issuer *breez_sdk_spark.TokenIssuer, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: issuer mint-token <amount>")
		return nil
	}

	amount, ok := new(big.Int).SetString(args[0], 10)
	if !ok {
		return fmt.Errorf("invalid amount: %s", args[0])
	}

	result, err := issuer.MintIssuerToken(breez_sdk_spark.MintIssuerTokenRequest{
		Amount: amount,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- burn-token ---

func handleBurnToken(issuer *breez_sdk_spark.TokenIssuer, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: issuer burn-token <amount>")
		return nil
	}

	amount, ok := new(big.Int).SetString(args[0], 10)
	if !ok {
		return fmt.Errorf("invalid amount: %s", args[0])
	}

	result, err := issuer.BurnIssuerToken(breez_sdk_spark.BurnIssuerTokenRequest{
		Amount: amount,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- freeze-token ---

func handleFreezeToken(issuer *breez_sdk_spark.TokenIssuer, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: issuer freeze-token <address>")
		return nil
	}

	result, err := issuer.FreezeIssuerToken(breez_sdk_spark.FreezeIssuerTokenRequest{
		Address: args[0],
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- unfreeze-token ---

func handleUnfreezeToken(issuer *breez_sdk_spark.TokenIssuer, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: issuer unfreeze-token <address>")
		return nil
	}

	result, err := issuer.UnfreezeIssuerToken(breez_sdk_spark.UnfreezeIssuerTokenRequest{
		Address: args[0],
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}
