package main

import (
	"flag"
	"fmt"
	"math/big"
	"sort"
	"strconv"
	"strings"
	"time"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// TokenAllowanceCmd represents a single token-allowances subcommand.
type TokenAllowanceCmd struct {
	Name        string
	Description string
	Run         func(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error
}

// TokenAllowanceCommandNames lists all token-allowances subcommand names (used for REPL completion).
var TokenAllowanceCommandNames = []string{
	"token-allowances create",
	"token-allowances revoke",
	"token-allowances list",
}

// BuildTokenAllowanceRegistry returns a map of token-allowances subcommand name to TokenAllowanceCmd.
func BuildTokenAllowanceRegistry() map[string]TokenAllowanceCmd {
	return map[string]TokenAllowanceCmd{
		"create": {
			Name:        "create",
			Description: "Grant a spender an allowance over this wallet's tokens",
			Run:         handleTokenAllowanceCreate,
		},
		"revoke": {
			Name:        "revoke",
			Description: "Revoke a token allowance this wallet granted",
			Run:         handleTokenAllowanceRevoke,
		},
		"list": {
			Name:        "list",
			Description: "List token allowances this wallet granted (owner) or received (spender)",
			Run:         handleTokenAllowanceList,
		},
	}
}

// DispatchTokenAllowanceCommand dispatches a token-allowances subcommand.
func DispatchTokenAllowanceCommand(args []string, sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance) {
	registry := BuildTokenAllowanceRegistry()

	if len(args) == 0 || args[0] == "help" {
		fmt.Println("\nToken allowances subcommands:")
		names := make([]string, 0, len(registry))
		for name := range registry {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			cmd := registry[name]
			fmt.Printf("  token-allowances %-24s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	subName := args[0]
	subArgs := args[1:]

	cmd, ok := registry[subName]
	if !ok {
		fmt.Printf("Unknown token-allowances subcommand: %s. Use 'token-allowances help' for available commands.\n", subName)
		return
	}

	if err := cmd.Run(sdk, rl, subArgs); err != nil {
		fmt.Printf("Error: %v\n", err)
	}
}

// ---------------------------------------------------------------------------
// Token allowances command handlers
// ---------------------------------------------------------------------------

// --- create ---

func handleTokenAllowanceCreate(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("token-allowances create", flag.ContinueOnError)
	maxPerPayment := fs.String("max-per-payment", "", "Limit on one payment, in token base units, or 'unlimited'")
	maxTotal := fs.String("max-total", "", "Limit on all payments together, in token base units, or 'unlimited'")
	var allowedRecipients stringSliceFlag
	fs.Var(&allowedRecipients, "r", "Spark address the spender may pay (repeatable, omit for any)")
	fs.Var(&allowedRecipients, "recipient", "Spark address the spender may pay")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 3 || *maxPerPayment == "" || *maxTotal == "" {
		fmt.Println("Usage: token-allowances create <spender_address> <token_identifier> <expiry_time> --max-per-payment <limit> --max-total <limit> [-r <recipient> ...]")
		return nil
	}

	expiryTime, err := strconv.ParseUint(positional[2], 10, 64)
	if err != nil {
		return fmt.Errorf("invalid expiry_time: %w", err)
	}

	mpp, err := parseTokenAllowanceLimit(*maxPerPayment)
	if err != nil {
		return err
	}
	mt, err := parseTokenAllowanceLimit(*maxTotal)
	if err != nil {
		return err
	}

	recipients := []string(allowedRecipients)
	if recipients == nil {
		recipients = []string{}
	}

	response, err := sdk.CreateTokenAllowance(breez_sdk_spark.CreateTokenAllowanceRequest{
		SpenderAddress:    positional[0],
		TokenIdentifier:   positional[1],
		MaxPerPayment:     mpp,
		MaxTotal:          mt,
		ExpiryTime:        expiryTime,
		AllowedRecipients: recipients,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(response.Allowance)
	return nil
}

// --- revoke ---

func handleTokenAllowanceRevoke(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: token-allowances revoke <allowance_id>")
		return nil
	}

	if err := liftError(sdk.RevokeTokenAllowance(breez_sdk_spark.RevokeTokenAllowanceRequest{
		AllowanceId: args[0],
	})); err != nil {
		return err
	}
	fmt.Println("Token allowance revoked")
	return nil
}

// --- list ---

func handleTokenAllowanceList(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("token-allowances list", flag.ContinueOnError)
	counterpartyAddress := fs.String("counterparty-address", "", "Only allowances with this counterparty Spark address")
	tokenIdentifier := fs.String("token-identifier", "", "Only allowances for this token")
	includeInactive := fs.String("include-inactive", "", "Include revoked, expired and exhausted allowances (true/false)")
	offset := fs.Uint("offset", 0, "Number of allowances to skip")
	limit := fs.Uint("limit", 0, "Maximum number of allowances to return")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	roleStr := "owner"
	if len(positional) > 0 {
		roleStr = positional[0]
	}

	role, err := parseTokenAllowanceRole(roleStr)
	if err != nil {
		return err
	}

	req := breez_sdk_spark.ListTokenAllowancesRequest{
		Role: role,
	}
	if *counterpartyAddress != "" {
		req.CounterpartyAddress = counterpartyAddress
	}
	if *tokenIdentifier != "" {
		req.TokenIdentifier = tokenIdentifier
	}
	if *includeInactive != "" {
		val := *includeInactive == "true"
		req.IncludeInactive = &val
	}
	if *offset > 0 {
		o := uint32(*offset)
		req.Offset = &o
	}
	if *limit > 0 {
		l := uint32(*limit)
		req.Limit = &l
	}

	response, err := sdk.ListTokenAllowances(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(response.Allowances)
	return nil
}

// ---------------------------------------------------------------------------
// Pull-payment retry loop
// ---------------------------------------------------------------------------

const (
	pullRetryInterval = 5 * time.Second
	pullRetryWindow   = 10 * time.Minute
)

func pullUntilSettled(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PreparePullPaymentResponse) (breez_sdk_spark.PullPaymentResponse, error) {
	deadline := time.Unix(int64(prepareResponse.ExpiryTime), 0).Add(pullRetryWindow)
	for {
		response, rawErr := sdk.PullPayment(breez_sdk_spark.PullPaymentRequest{
			PrepareResponse: prepareResponse,
		})
		err := liftError(rawErr)
		if !mayStillSettle(response, err) || time.Now().After(deadline) {
			return response, err
		}
		if err != nil {
			fmt.Printf("Sending the pull failed (%v), trying again in %ds\n", err, int(pullRetryInterval.Seconds()))
		} else {
			fmt.Printf("The pull %s is pending, checking again in %ds\n", response.TxHash, int(pullRetryInterval.Seconds()))
		}
		time.Sleep(pullRetryInterval)
	}
}

func mayStillSettle(response breez_sdk_spark.PullPaymentResponse, err error) bool {
	if err == nil {
		return response.Status == breez_sdk_spark.PaymentStatusPending
	}
	errMsg := err.Error()
	return strings.Contains(errMsg, "NetworkError") || strings.Contains(errMsg, "SparkError")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

func parseTokenAllowanceLimit(s string) (breez_sdk_spark.TokenAllowanceLimit, error) {
	if strings.EqualFold(s, "unlimited") {
		return breez_sdk_spark.TokenAllowanceLimitUnlimited{}, nil
	}
	amount, ok := new(big.Int).SetString(s, 10)
	if !ok {
		return nil, fmt.Errorf("invalid limit '%s': expected an amount or 'unlimited'", s)
	}
	return breez_sdk_spark.TokenAllowanceLimitAmount{Amount: amount}, nil
}

func parseTokenAllowanceRole(s string) (breez_sdk_spark.TokenAllowanceRole, error) {
	switch strings.ToLower(s) {
	case "owner":
		return breez_sdk_spark.TokenAllowanceRoleOwner, nil
	case "spender":
		return breez_sdk_spark.TokenAllowanceRoleSpender, nil
	default:
		return 0, fmt.Errorf("invalid role '%s': expected 'owner' or 'spender'", s)
	}
}
