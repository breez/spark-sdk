package main

import (
	"flag"
	"fmt"
	"math/big"
	"strconv"
	"strings"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// BridgingCommandNames lists all bridging subcommand names (used for REPL completion).
var BridgingCommandNames = []string{
	"bridging create",
	"bridging from-cash-app",
	"bridging to-cash-app",
	"bridging status",
}

// BuildBridgingCommandRegistry returns a map of subcommand name -> Command.
func BuildBridgingCommandRegistry() map[string]Command {
	return map[string]Command{
		"create":        {Name: "create", Description: "Create a bridge that pays a recipient from another chain or over Lightning", Run: handleBridgingCreate},
		"from-cash-app": {Name: "from-cash-app", Description: "Bridge from Cash App to USDC/USDT on an external chain", Run: handleBridgingFromCashApp},
		"to-cash-app":   {Name: "to-cash-app", Description: "Bridge USDC/USDT from an external chain to a Cash App user", Run: handleBridgingToCashApp},
		"status":        {Name: "status", Description: "Read a bridge's status", Run: handleBridgingStatus},
	}
}

// DispatchBridgingCommand parses the subcommand and dispatches it.
func DispatchBridgingCommand(args []string, sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance) {
	if len(args) == 0 || args[0] == "help" {
		registry := BuildBridgingCommandRegistry()
		fmt.Println("\nBridging commands:")
		for _, name := range []string{"create", "from-cash-app", "to-cash-app", "status"} {
			cmd := registry[name]
			fmt.Printf("  bridging %-20s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	registry := BuildBridgingCommandRegistry()
	subCmd := args[0]
	subArgs := args[1:]
	if cmd, ok := registry[subCmd]; ok {
		if err := cmd.Run(sdk, rl, subArgs); err != nil {
			fmt.Printf("Error: %v\n", err)
		}
	} else {
		fmt.Printf("Unknown bridging command: %s. Use 'bridging help' for available commands.\n", subCmd)
	}
}

// --- bridging create ---

func handleBridgingCreate(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("bridging create", flag.ContinueOnError)
	amountStr := fs.String("amount", "", "Amount in stablecoin base units")
	refundAddress := fs.String("refund-address", "", "The payer's address on the source chain, refunded if delivery fails")
	feesIncluded := fs.Bool("fees-included", false, "Deduct fees from the amount instead of adding them on top")
	maxSlippageStr := fs.String("max-slippage-bps", "", "Maximum slippage in basis points")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 1 || *amountStr == "" {
		fmt.Println("Usage: bridging create <recipient> --amount <amount> [--refund-address <address>] [--fees-included] [--max-slippage-bps <bps>]")
		return nil
	}
	recipient := positional[0]

	client, err := sdk.GetBridgingClient()
	if err = liftError(err); err != nil {
		return err
	}

	routesResp, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
		Recipient: recipient,
	})
	if err = liftError(err); err != nil {
		return err
	}

	route, err := chooseBridgeRoute(rl, routesResp.Routes)
	if err != nil {
		return err
	}

	amount, ok := new(big.Int).SetString(*amountStr, 10)
	if !ok {
		return fmt.Errorf("invalid amount: %s", *amountStr)
	}

	req := breez_sdk_spark.CreateBridgeRequest{
		Recipient: recipient,
		Route:     route,
		Amount:    amount,
	}
	if *refundAddress != "" {
		req.RefundAddress = refundAddress
	}
	if *feesIncluded {
		fp := breez_sdk_spark.FeePolicyFeesIncluded
		req.FeePolicy = &fp
	}
	if *maxSlippageStr != "" {
		val, err := strconv.ParseUint(*maxSlippageStr, 10, 32)
		if err != nil {
			return fmt.Errorf("invalid max slippage: %s", *maxSlippageStr)
		}
		val32 := uint32(val)
		req.MaxSlippageBps = &val32
	}

	response, err := client.CreateBridge(req)
	if err = liftError(err); err != nil {
		return err
	}

	fmt.Println("Ask the payer to pay:")
	fmt.Println(response.PaymentRequest)
	printValue(response)
	printBridgeTracking(response)
	return nil
}

// --- bridging from-cash-app ---

func handleBridgingFromCashApp(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("bridging from-cash-app", flag.ContinueOnError)
	amountStr := fs.String("amount", "", "Amount in the destination stablecoin's base units (6 decimals for USDC/USDT)")
	feesIncluded := fs.Bool("fees-included", false, "Deduct fees from the amount instead of adding them on top")
	maxSlippageStr := fs.String("max-slippage-bps", "", "Maximum slippage in basis points")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 1 || *amountStr == "" {
		fmt.Println("Usage: bridging from-cash-app <recipient> --amount <amount> [--fees-included] [--max-slippage-bps <bps>]")
		return nil
	}
	recipient := positional[0]

	client, err := sdk.GetBridgingClient()
	if err = liftError(err); err != nil {
		return err
	}

	routesResp, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
		Recipient: recipient,
	})
	if err = liftError(err); err != nil {
		return err
	}

	var lightningRoutes []breez_sdk_spark.BridgeRoutePair
	for _, route := range routesResp.Routes {
		if route.Source.Chain == "lightning" {
			lightningRoutes = append(lightningRoutes, route)
		}
	}

	route, err := chooseBridgeRoute(rl, lightningRoutes)
	if err != nil {
		return err
	}

	amount, ok := new(big.Int).SetString(*amountStr, 10)
	if !ok {
		return fmt.Errorf("invalid amount: %s", *amountStr)
	}

	var feePolicy *breez_sdk_spark.FeePolicy
	if *feesIncluded {
		fp := breez_sdk_spark.FeePolicyFeesIncluded
		feePolicy = &fp
	}

	req := breez_sdk_spark.CreateBridgeRequest{
		Recipient: recipient,
		Route:     route,
		Amount:    amount,
		FeePolicy: feePolicy,
	}
	if *maxSlippageStr != "" {
		val, err := strconv.ParseUint(*maxSlippageStr, 10, 32)
		if err != nil {
			return fmt.Errorf("invalid max slippage: %s", *maxSlippageStr)
		}
		val32 := uint32(val)
		req.MaxSlippageBps = &val32
	}

	response, err := client.BridgeFromCashApp(req)
	if err = liftError(err); err != nil {
		return err
	}

	info := response.Info
	fmt.Println("Open this URL in a browser to complete the purchase:")
	fmt.Println(response.PaymentRequest)
	serviceFeeAsset := serviceFeeDenomination(info.ServiceFeeAsset, info.ServiceFeeAssetDecimals)
	fmt.Printf("Deposit ~%v sats; recipient receives ~%v %s, service fee %v %s, expires %v\n",
		info.DepositAmount, info.ExpectedReceivedAmount, info.DestinationAsset,
		info.ServiceFeeAmount, serviceFeeAsset, info.ExpiresAt)
	printBridgeTracking(response)
	return nil
}

// --- bridging to-cash-app ---

func handleBridgingToCashApp(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("bridging to-cash-app", flag.ContinueOnError)
	amountStr := fs.String("amount", "", "Amount in the source stablecoin's base units (6 decimals for USDC/USDT)")
	refundAddress := fs.String("refund-address", "", "The payer's address on the source chain, refunded if delivery fails")
	feesIncluded := fs.Bool("fees-included", false, "Deduct fees from the amount instead of adding them on top")
	maxSlippageStr := fs.String("max-slippage-bps", "", "Maximum slippage in basis points")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 1 || *amountStr == "" || *refundAddress == "" {
		fmt.Println("Usage: bridging to-cash-app <recipient> --amount <amount> --refund-address <address> [--fees-included] [--max-slippage-bps <bps>]")
		return nil
	}
	recipient := positional[0]

	client, err := sdk.GetBridgingClient()
	if err = liftError(err); err != nil {
		return err
	}

	routesResp, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
		Recipient: recipient,
	})
	if err = liftError(err); err != nil {
		return err
	}

	route, err := chooseBridgeRoute(rl, routesResp.Routes)
	if err != nil {
		return err
	}

	amount, ok := new(big.Int).SetString(*amountStr, 10)
	if !ok {
		return fmt.Errorf("invalid amount: %s", *amountStr)
	}

	var feePolicy *breez_sdk_spark.FeePolicy
	if *feesIncluded {
		fp := breez_sdk_spark.FeePolicyFeesIncluded
		feePolicy = &fp
	}

	req := breez_sdk_spark.CreateBridgeRequest{
		Recipient:     recipient,
		Route:         route,
		Amount:        amount,
		RefundAddress: refundAddress,
		FeePolicy:     feePolicy,
	}
	if *maxSlippageStr != "" {
		val, err := strconv.ParseUint(*maxSlippageStr, 10, 32)
		if err != nil {
			return fmt.Errorf("invalid max slippage: %s", *maxSlippageStr)
		}
		val32 := uint32(val)
		req.MaxSlippageBps = &val32
	}

	response, err := client.BridgeToCashApp(req)
	if err = liftError(err); err != nil {
		return err
	}

	info := response.Info
	fmt.Println("Ask the payer to pay:")
	fmt.Println(response.PaymentRequest)
	serviceFeeAsset := serviceFeeDenomination(info.ServiceFeeAsset, info.ServiceFeeAssetDecimals)
	fmt.Printf("Deposit %v %s on %s to %s. Recipient receives ~%v sats, service fee %v %s, expires at %v\n",
		info.DepositAmount, route.Source.Asset, route.Source.Chain, info.DepositAddress,
		info.ExpectedReceivedAmount, info.ServiceFeeAmount, serviceFeeAsset, info.ExpiresAt)
	printBridgeTracking(response)
	return nil
}

// --- bridging status ---

func handleBridgingStatus(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("bridging status", flag.ContinueOnError)
	statusToken := fs.String("status-token", "", "The status token bridging create returned")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 1 {
		fmt.Println("Usage: bridging status <bridge_id> [--status-token <token>]")
		return nil
	}

	client, err := sdk.GetBridgingClient()
	if err = liftError(err); err != nil {
		return err
	}

	req := breez_sdk_spark.GetBridgeStatusRequest{
		BridgeId: positional[0],
	}
	if *statusToken != "" {
		req.StatusToken = statusToken
	}

	response, err := client.GetBridgeStatus(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(response)
	return nil
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

func chooseBridgeRoute(rl *readline.Instance, routes []breez_sdk_spark.BridgeRoutePair) (breez_sdk_spark.BridgeRoutePair, error) {
	if len(routes) == 0 {
		return breez_sdk_spark.BridgeRoutePair{}, fmt.Errorf("no bridge routes available")
	}

	if len(routes) == 1 {
		route := routes[0]
		fmt.Printf("Auto-selected route: %s\n", bridgeRouteLabel(route))
		return route, nil
	}

	fmt.Println("Available bridge routes:")
	for i, route := range routes {
		fmt.Printf("  %d. %s\n", i+1, bridgeRouteLabel(route))
	}

	line, err := readlinePrompt(rl, "Select route: ")
	if err != nil {
		return breez_sdk_spark.BridgeRoutePair{}, err
	}
	choice, err := strconv.Atoi(strings.TrimSpace(line))
	if err != nil {
		return breez_sdk_spark.BridgeRoutePair{}, fmt.Errorf("invalid selection")
	}
	idx := choice - 1
	if idx < 0 || idx >= len(routes) {
		return breez_sdk_spark.BridgeRoutePair{}, fmt.Errorf("selection out of range")
	}
	return routes[idx], nil
}

func bridgeRouteLabel(route breez_sdk_spark.BridgeRoutePair) string {
	return fmt.Sprintf("%s -> %s [%v]",
		bridgeSideLabel(route.Source),
		bridgeSideLabel(route.Destination),
		route.Provider)
}

func bridgeSideLabel(side breez_sdk_spark.BridgeRoutePairSide) string {
	return fmt.Sprintf("%s on %s%s",
		side.Asset, side.Chain, maybeTruncateAddress(side.ContractAddress))
}

func printBridgeTracking(response breez_sdk_spark.CreateBridgeResponse) {
	token := ""
	if response.StatusToken != nil {
		token = fmt.Sprintf(" --status-token %s", *response.StatusToken)
	}
	fmt.Printf("Track it with: bridging status %s%s\n", response.BridgeId, token)
}
