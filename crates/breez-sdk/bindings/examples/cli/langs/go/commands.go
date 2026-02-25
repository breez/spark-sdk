package main

import (
	"flag"
	"fmt"
	"math/big"
	"sort"
	"strconv"
	"strings"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// Command represents a single CLI command.
type Command struct {
	Name        string
	Description string
	Run         func(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error
}

// CommandNames lists all top-level command names (used for REPL completion).
var CommandNames = []string{
	"get-info",
	"get-payment",
	"sync",
	"list-payments",
	"receive",
	"pay",
	"lnurl-pay",
	"lnurl-withdraw",
	"lnurl-auth",
	"claim-htlc-payment",
	"claim-deposit",
	"parse",
	"refund-deposit",
	"list-unclaimed-deposits",
	"buy-bitcoin",
	"check-lightning-address-available",
	"get-lightning-address",
	"register-lightning-address",
	"delete-lightning-address",
	"list-fiat-currencies",
	"list-fiat-rates",
	"recommended-fees",
	"get-tokens-metadata",
	"fetch-conversion-limits",
	"get-user-settings",
	"set-user-settings",
	"get-spark-status",
}

// BuildCommandRegistry returns a map of command name → Command.
func BuildCommandRegistry() map[string]Command {
	return map[string]Command{
		"get-info":                          {Name: "get-info", Description: "Get balance information", Run: handleGetInfo},
		"get-payment":                       {Name: "get-payment", Description: "Get the payment with the given ID", Run: handleGetPayment},
		"sync":                              {Name: "sync", Description: "Sync wallet state", Run: handleSync},
		"list-payments":                     {Name: "list-payments", Description: "List payments", Run: handleListPayments},
		"receive":                           {Name: "receive", Description: "Receive a payment", Run: handleReceive},
		"pay":                               {Name: "pay", Description: "Pay the given payment request", Run: handlePay},
		"lnurl-pay":                         {Name: "lnurl-pay", Description: "Pay using LNURL", Run: handleLnurlPay},
		"lnurl-withdraw":                    {Name: "lnurl-withdraw", Description: "Withdraw using LNURL", Run: handleLnurlWithdraw},
		"lnurl-auth":                        {Name: "lnurl-auth", Description: "Authenticate using LNURL", Run: handleLnurlAuth},
		"claim-htlc-payment":                {Name: "claim-htlc-payment", Description: "Claim an HTLC payment", Run: handleClaimHtlcPayment},
		"claim-deposit":                     {Name: "claim-deposit", Description: "Claim an on-chain deposit", Run: handleClaimDeposit},
		"parse":                             {Name: "parse", Description: "Parse an input (invoice, address, LNURL)", Run: handleParse},
		"refund-deposit":                    {Name: "refund-deposit", Description: "Refund an on-chain deposit", Run: handleRefundDeposit},
		"list-unclaimed-deposits":           {Name: "list-unclaimed-deposits", Description: "List unclaimed on-chain deposits", Run: handleListUnclaimedDeposits},
		"buy-bitcoin":                       {Name: "buy-bitcoin", Description: "Buy Bitcoin via MoonPay", Run: handleBuyBitcoin},
		"check-lightning-address-available": {Name: "check-lightning-address-available", Description: "Check if a lightning address username is available", Run: handleCheckLightningAddress},
		"get-lightning-address":             {Name: "get-lightning-address", Description: "Get registered lightning address", Run: handleGetLightningAddress},
		"register-lightning-address":        {Name: "register-lightning-address", Description: "Register a lightning address", Run: handleRegisterLightningAddress},
		"delete-lightning-address":          {Name: "delete-lightning-address", Description: "Delete lightning address", Run: handleDeleteLightningAddress},
		"list-fiat-currencies":              {Name: "list-fiat-currencies", Description: "List fiat currencies", Run: handleListFiatCurrencies},
		"list-fiat-rates":                   {Name: "list-fiat-rates", Description: "List available fiat rates", Run: handleListFiatRates},
		"recommended-fees":                  {Name: "recommended-fees", Description: "Get recommended BTC fees", Run: handleRecommendedFees},
		"get-tokens-metadata":               {Name: "get-tokens-metadata", Description: "Get metadata for token(s)", Run: handleGetTokensMetadata},
		"fetch-conversion-limits":           {Name: "fetch-conversion-limits", Description: "Fetch conversion limits for a token", Run: handleFetchConversionLimits},
		"get-user-settings":                 {Name: "get-user-settings", Description: "Get user settings", Run: handleGetUserSettings},
		"set-user-settings":                 {Name: "set-user-settings", Description: "Update user settings", Run: handleSetUserSettings},
		"get-spark-status":                  {Name: "get-spark-status", Description: "Get Spark network service status", Run: handleGetSparkStatus},
	}
}

// PrintHelp prints available commands.
func PrintHelp(registry map[string]Command) {
	fmt.Println("\nAvailable commands:\n")
	names := make([]string, 0, len(registry))
	for name := range registry {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		cmd := registry[name]
		fmt.Printf("  %-40s %s\n", name, cmd.Description)
	}
	fmt.Printf("\n  %-40s %s\n", "issuer <subcommand>", "Token issuer commands (use 'issuer help' for details)")
	fmt.Printf("  %-40s %s\n", "exit / quit", "Exit the CLI")
	fmt.Printf("  %-40s %s\n", "help", "Show this help message")
	fmt.Println()
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

// --- get-info ---

func handleGetInfo(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- get-payment ---

func handleGetPayment(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: get-payment <payment_id>")
		return nil
	}

	result, err := sdk.GetPayment(breez_sdk_spark.GetPaymentRequest{PaymentId: args[0]})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- sync ---

func handleSync(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.SyncWallet(breez_sdk_spark.SyncWalletRequest{})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- list-payments ---

func handleListPayments(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("list-payments", flag.ContinueOnError)
	limit := fs.Uint("l", 10, "Number of payments to show")
	fs.UintVar(limit, "limit", 10, "Number of payments to show")
	offset := fs.Uint("o", 0, "Number of payments to skip")
	fs.UintVar(offset, "offset", 0, "Number of payments to skip")
	if err := fs.Parse(args); err != nil {
		return err
	}

	limitU32 := uint32(*limit)
	offsetU32 := uint32(*offset)

	result, err := sdk.ListPayments(breez_sdk_spark.ListPaymentsRequest{
		Limit:  &limitU32,
		Offset: &offsetU32,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- receive ---

func handleReceive(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("receive", flag.ContinueOnError)
	method := fs.String("m", "", "Payment method: sparkaddress, bitcoin, bolt11")
	fs.StringVar(method, "method", "", "Payment method")
	description := fs.String("d", "", "Optional description (bolt11 only)")
	fs.StringVar(description, "description", "", "Optional description")
	amountStr := fs.String("a", "", "Amount (bolt11 only)")
	fs.StringVar(amountStr, "amount", "", "Amount")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *method == "" {
		fmt.Println("Usage: receive -m <method> [options]")
		fmt.Println("Methods: sparkaddress, bitcoin, bolt11")
		return nil
	}

	var paymentMethod breez_sdk_spark.ReceivePaymentMethod

	switch strings.ToLower(*method) {
	case "sparkaddress":
		paymentMethod = breez_sdk_spark.ReceivePaymentMethodSparkAddress{}

	case "bitcoin":
		paymentMethod = breez_sdk_spark.ReceivePaymentMethodBitcoinAddress{}

	case "bolt11":
		pm := breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
			Description: stringOrDefault(description, ""),
		}
		if *amountStr != "" {
			amountVal, err := strconv.ParseUint(*amountStr, 10, 64)
			if err != nil {
				return fmt.Errorf("invalid amount: %s", *amountStr)
			}
			pm.AmountSats = &amountVal
		}
		paymentMethod = pm

	default:
		fmt.Printf("Invalid payment method: %s\n", *method)
		fmt.Println("Available methods: sparkaddress, bitcoin, bolt11")
		return nil
	}

	result, err := sdk.ReceivePayment(breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: paymentMethod,
	})
	if err = liftError(err); err != nil {
		return err
	}

	printValue(result)
	return nil
}

// --- pay ---

func handlePay(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("pay", flag.ContinueOnError)
	paymentRequest := fs.String("r", "", "The payment request to pay")
	fs.StringVar(paymentRequest, "payment-request", "", "The payment request")
	amountStr := fs.String("a", "", "Optional amount")
	fs.StringVar(amountStr, "amount", "", "Optional amount")
	tokenId := fs.String("t", "", "Optional token identifier")
	fs.StringVar(tokenId, "token-identifier", "", "Optional token identifier")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *paymentRequest == "" {
		fmt.Println("Usage: pay -r <payment_request> [-a <amount>] [-t <token_identifier>]")
		return nil
	}

	req := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: *paymentRequest,
	}

	if *amountStr != "" {
		amount, ok := new(big.Int).SetString(*amountStr, 10)
		if !ok {
			return fmt.Errorf("invalid amount: %s", *amountStr)
		}
		req.Amount = &amount
	}

	if *tokenId != "" {
		req.TokenIdentifier = tokenId
	}

	prepareResponse, err := sdk.PrepareSendPayment(req)
	if err = liftError(err); err != nil {
		return fmt.Errorf("failed to prepare payment: %w", err)
	}

	// Payment options
	paymentOptions, err := readPaymentOptions(prepareResponse.PaymentMethod, rl)
	if err != nil {
		return err
	}

	sendReq := breez_sdk_spark.SendPaymentRequest{
		PrepareResponse: prepareResponse,
		Options:         paymentOptions,
	}

	result, err := sdk.SendPayment(sendReq)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- lnurl-pay ---

func handleLnurlPay(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("lnurl-pay", flag.ContinueOnError)
	comment := fs.String("c", "", "Comment for the invoice")
	fs.StringVar(comment, "comment", "", "Comment")
	validateStr := fs.String("v", "", "Validate success action URL (true/false)")
	fs.StringVar(validateStr, "validate", "", "Validate success URL")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 1 {
		fmt.Println("Usage: lnurl-pay <lnurl> [options]")
		return nil
	}
	lnurl := positional[0]

	parsed, err := sdk.Parse(lnurl)
	if err = liftError(err); err != nil {
		return err
	}

	var payRequest breez_sdk_spark.LnurlPayRequestDetails
	switch v := parsed.(type) {
	case breez_sdk_spark.InputTypeLnurlPay:
		payRequest = v.Field0
	case breez_sdk_spark.InputTypeLightningAddress:
		payRequest = v.Field0.PayRequest
	default:
		return fmt.Errorf("input is not an LNURL-pay or lightning address")
	}

	printValue(payRequest)

	amountLine, err := readlinePrompt(rl, "Amount to pay (sats): ")
	if err != nil {
		return err
	}
	amountSats, err := strconv.ParseUint(strings.TrimSpace(amountLine), 10, 64)
	if err != nil {
		return fmt.Errorf("invalid amount: %w", err)
	}

	prepareReq := breez_sdk_spark.PrepareLnurlPayRequest{
		AmountSats: amountSats,
		PayRequest: payRequest,
	}
	if *comment != "" {
		prepareReq.Comment = comment
	}
	if *validateStr != "" {
		val := *validateStr == "true"
		prepareReq.ValidateSuccessActionUrl = &val
	}

	prepareResponse, err := sdk.PrepareLnurlPay(prepareReq)
	if err = liftError(err); err != nil {
		return err
	}

	printValue(prepareResponse)

	result, err := sdk.LnurlPay(breez_sdk_spark.LnurlPayRequest{
		PrepareResponse: prepareResponse,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- lnurl-withdraw ---

func handleLnurlWithdraw(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("lnurl-withdraw", flag.ContinueOnError)
	timeoutSecs := fs.Uint("timeout", 0, "Completion timeout in seconds")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 1 {
		fmt.Println("Usage: lnurl-withdraw <lnurl> [--timeout N]")
		return nil
	}

	parsed, err := sdk.Parse(positional[0])
	if err = liftError(err); err != nil {
		return err
	}

	withdrawData, ok := parsed.(breez_sdk_spark.InputTypeLnurlWithdraw)
	if !ok {
		return fmt.Errorf("input is not an LNURL-withdraw")
	}

	printValue(withdrawData.Field0)

	amountLine, err := readlinePrompt(rl, "Amount to withdraw (sats): ")
	if err != nil {
		return err
	}
	amountSats, err := strconv.ParseUint(strings.TrimSpace(amountLine), 10, 64)
	if err != nil {
		return fmt.Errorf("invalid amount: %w", err)
	}

	req := breez_sdk_spark.LnurlWithdrawRequest{
		AmountSats:      amountSats,
		WithdrawRequest: withdrawData.Field0,
	}
	if *timeoutSecs > 0 {
		t := uint32(*timeoutSecs)
		req.CompletionTimeoutSecs = &t
	}

	result, err := sdk.LnurlWithdraw(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- lnurl-auth ---

func handleLnurlAuth(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: lnurl-auth <lnurl>")
		return nil
	}

	parsed, err := sdk.Parse(args[0])
	if err = liftError(err); err != nil {
		return err
	}

	authData, ok := parsed.(breez_sdk_spark.InputTypeLnurlAuth)
	if !ok {
		return fmt.Errorf("input is not an LNURL-auth")
	}

	result, err := sdk.LnurlAuth(authData.Field0)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- claim-htlc-payment ---

func handleClaimHtlcPayment(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: claim-htlc-payment <preimage>")
		return nil
	}

	result, err := sdk.ClaimHtlcPayment(breez_sdk_spark.ClaimHtlcPaymentRequest{
		Preimage: args[0],
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- claim-deposit ---

func handleClaimDeposit(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("claim-deposit", flag.ContinueOnError)
	feeSat := fs.Uint64("fee-sat", 0, "Max fee in sats (fixed)")
	satPerVbyte := fs.Uint64("sat-per-vbyte", 0, "Max fee per vbyte (rate)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 2 {
		fmt.Println("Usage: claim-deposit <txid> <vout> [--fee-sat N | --sat-per-vbyte N]")
		return nil
	}

	txid := positional[0]
	vout, err := strconv.ParseUint(positional[1], 10, 32)
	if err != nil {
		return fmt.Errorf("invalid vout: %w", err)
	}

	var maxFee breez_sdk_spark.MaxFee
	if *feeSat > 0 && *satPerVbyte > 0 {
		fmt.Println("Cannot specify both --fee-sat and --sat-per-vbyte")
		return nil
	} else if *feeSat > 0 {
		maxFee = breez_sdk_spark.MaxFeeFixed{Amount: *feeSat}
	} else if *satPerVbyte > 0 {
		maxFee = breez_sdk_spark.MaxFeeRate{SatPerVbyte: *satPerVbyte}
	}

	req := breez_sdk_spark.ClaimDepositRequest{
		Txid: txid,
		Vout: uint32(vout),
	}
	if maxFee != nil {
		req.MaxFee = &maxFee
	}

	result, err := sdk.ClaimDeposit(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- parse ---

func handleParse(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: parse <input>")
		return nil
	}

	result, err := sdk.Parse(args[0])
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- refund-deposit ---

func handleRefundDeposit(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("refund-deposit", flag.ContinueOnError)
	feeSat := fs.Uint64("fee-sat", 0, "Fee in sats (fixed)")
	satPerVbyte := fs.Uint64("sat-per-vbyte", 0, "Fee per vbyte (rate)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 3 {
		fmt.Println("Usage: refund-deposit <txid> <vout> <destination_address> [--fee-sat N | --sat-per-vbyte N]")
		return nil
	}

	txid := positional[0]
	vout, err := strconv.ParseUint(positional[1], 10, 32)
	if err != nil {
		return fmt.Errorf("invalid vout: %w", err)
	}
	destAddr := positional[2]

	var fee breez_sdk_spark.Fee
	if *feeSat > 0 && *satPerVbyte > 0 {
		fmt.Println("Cannot specify both --fee-sat and --sat-per-vbyte")
		return nil
	} else if *feeSat > 0 {
		fee = breez_sdk_spark.FeeFixed{Amount: *feeSat}
	} else if *satPerVbyte > 0 {
		fee = breez_sdk_spark.FeeRate{SatPerVbyte: *satPerVbyte}
	} else {
		fmt.Println("Must specify either --fee-sat or --sat-per-vbyte")
		return nil
	}

	result, err := sdk.RefundDeposit(breez_sdk_spark.RefundDepositRequest{
		Txid:               txid,
		Vout:               uint32(vout),
		DestinationAddress: destAddr,
		Fee:                fee,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- list-unclaimed-deposits ---

func handleListUnclaimedDeposits(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.ListUnclaimedDeposits(breez_sdk_spark.ListUnclaimedDepositsRequest{})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- buy-bitcoin ---

func handleBuyBitcoin(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("buy-bitcoin", flag.ContinueOnError)
	lockedAmount := fs.Uint64("amount", 0, "Lock purchase to this amount in sats")
	redirectUrl := fs.String("redirect-url", "", "Redirect URL after purchase")
	if err := fs.Parse(args); err != nil {
		return err
	}

	req := breez_sdk_spark.BuyBitcoinRequest{}
	if *lockedAmount > 0 {
		req.LockedAmountSat = lockedAmount
	}
	if *redirectUrl != "" {
		req.RedirectUrl = redirectUrl
	}

	result, err := sdk.BuyBitcoin(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- check-lightning-address-available ---

func handleCheckLightningAddress(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: check-lightning-address-available <username>")
		return nil
	}

	available, err := sdk.CheckLightningAddressAvailable(breez_sdk_spark.CheckLightningAddressRequest{
		Username: args[0],
	})
	if err = liftError(err); err != nil {
		return err
	}
	if available {
		fmt.Printf("Username '%s' is available\n", args[0])
	} else {
		fmt.Printf("Username '%s' is NOT available\n", args[0])
	}
	return nil
}

// --- get-lightning-address ---

func handleGetLightningAddress(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.GetLightningAddress()
	if err = liftError(err); err != nil {
		return err
	}
	if result == nil {
		fmt.Println("No lightning address registered")
	} else {
		printValue(result)
	}
	return nil
}

// --- register-lightning-address ---

func handleRegisterLightningAddress(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("register-lightning-address", flag.ContinueOnError)
	description := fs.String("d", "", "Optional description")
	fs.StringVar(description, "description", "", "Optional description")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 1 {
		fmt.Println("Usage: register-lightning-address <username> [-d <description>]")
		return nil
	}

	req := breez_sdk_spark.RegisterLightningAddressRequest{
		Username: positional[0],
	}
	if *description != "" {
		req.Description = description
	}

	result, err := sdk.RegisterLightningAddress(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- delete-lightning-address ---

func handleDeleteLightningAddress(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	if err := liftError(sdk.DeleteLightningAddress()); err != nil {
		return err
	}
	fmt.Println("Lightning address deleted")
	return nil
}

// --- list-fiat-currencies ---

func handleListFiatCurrencies(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.ListFiatCurrencies()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- list-fiat-rates ---

func handleListFiatRates(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.ListFiatRates()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- recommended-fees ---

func handleRecommendedFees(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.RecommendedFees()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- get-tokens-metadata ---

func handleGetTokensMetadata(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: get-tokens-metadata <token_id> [<token_id2> ...]")
		return nil
	}

	result, err := sdk.GetTokensMetadata(breez_sdk_spark.GetTokensMetadataRequest{
		TokenIdentifiers: args,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- fetch-conversion-limits ---

func handleFetchConversionLimits(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("fetch-conversion-limits", flag.ContinueOnError)
	fromBitcoin := fs.Bool("from-bitcoin", false, "Convert from bitcoin to token")
	tokenId := fs.String("token", "", "Token identifier (required)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *tokenId == "" {
		fmt.Println("Usage: fetch-conversion-limits --token <token_id> [--from-bitcoin]")
		return nil
	}

	var convType breez_sdk_spark.ConversionType
	if *fromBitcoin {
		convType = breez_sdk_spark.ConversionTypeFromBitcoin{}
	} else {
		convType = breez_sdk_spark.ConversionTypeToBitcoin{FromTokenIdentifier: *tokenId}
	}

	req := breez_sdk_spark.FetchConversionLimitsRequest{
		ConversionType: convType,
	}
	if *fromBitcoin {
		req.TokenIdentifier = tokenId
	}

	result, err := sdk.FetchConversionLimits(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- get-user-settings ---

func handleGetUserSettings(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.GetUserSettings()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- set-user-settings ---

func handleSetUserSettings(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("set-user-settings", flag.ContinueOnError)
	privateMode := fs.String("spark-private-mode", "", "Enable spark private mode (true/false)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	req := breez_sdk_spark.UpdateUserSettingsRequest{}
	if *privateMode != "" {
		val := *privateMode == "true"
		req.SparkPrivateModeEnabled = &val
	}

	if err := liftError(sdk.UpdateUserSettings(req)); err != nil {
		return err
	}
	fmt.Println("User settings updated")
	return nil
}

// --- get-spark-status ---

func handleGetSparkStatus(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := breez_sdk_spark.GetSparkStatus()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// ---------------------------------------------------------------------------
// readPaymentOptions — interactive fee/option selection
// ---------------------------------------------------------------------------

func readPaymentOptions(paymentMethod breez_sdk_spark.SendPaymentMethod, rl *readline.Instance) (*breez_sdk_spark.SendPaymentOptions, error) {
	switch pm := paymentMethod.(type) {
	case breez_sdk_spark.SendPaymentMethodBitcoinAddress:
		feeQuote := pm.FeeQuote
		fastFee := feeQuote.SpeedFast.UserFeeSat + feeQuote.SpeedFast.L1BroadcastFeeSat
		mediumFee := feeQuote.SpeedMedium.UserFeeSat + feeQuote.SpeedMedium.L1BroadcastFeeSat
		slowFee := feeQuote.SpeedSlow.UserFeeSat + feeQuote.SpeedSlow.L1BroadcastFeeSat
		fmt.Println("Please choose payment fee:")
		fmt.Printf("1. Fast: %d sats\n", fastFee)
		fmt.Printf("2. Medium: %d sats\n", mediumFee)
		fmt.Printf("3. Slow: %d sats\n", slowFee)
		line, err := readlineWithDefault(rl, "Choose (1/2/3): ", "1")
		if err != nil {
			return nil, err
		}
		var speed breez_sdk_spark.OnchainConfirmationSpeed
		switch strings.TrimSpace(line) {
		case "1":
			speed = breez_sdk_spark.OnchainConfirmationSpeedFast
		case "2":
			speed = breez_sdk_spark.OnchainConfirmationSpeedMedium
		case "3":
			speed = breez_sdk_spark.OnchainConfirmationSpeedSlow
		default:
			return nil, fmt.Errorf("invalid confirmation speed")
		}
		var opts breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBitcoinAddress{
			ConfirmationSpeed: speed,
		}
		return &opts, nil

	case breez_sdk_spark.SendPaymentMethodBolt11Invoice:
		if pm.SparkTransferFeeSats != nil {
			fmt.Println("Choose payment option:")
			fmt.Printf("1. Spark transfer fee: %d sats\n", *pm.SparkTransferFeeSats)
			fmt.Printf("2. Lightning fee: %d sats\n", pm.LightningFeeSats)
			line, err := readlineWithDefault(rl, "Choose (1/2): ", "1")
			if err != nil {
				return nil, err
			}
			if strings.TrimSpace(line) == "1" {
				var opts breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
					PreferSpark: true,
				}
				return &opts, nil
			}
		}
		var opts breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
			PreferSpark: false,
		}
		return &opts, nil

	case breez_sdk_spark.SendPaymentMethodSparkAddress:
		// No options for Spark address payments
		return nil, nil
	}

	return nil, nil
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// readlineWithDefault reads a line with a prompt, returning defaultVal if input is empty.
func readlineWithDefault(rl *readline.Instance, prompt, defaultVal string) (string, error) {
	rl.SetPrompt(prompt)
	line, err := rl.Readline()
	if err != nil {
		return "", err
	}
	if strings.TrimSpace(line) == "" {
		return defaultVal, nil
	}
	return line, nil
}

// readlinePrompt reads a line with a custom prompt.
func readlinePrompt(rl *readline.Instance, prompt string) (string, error) {
	rl.SetPrompt(prompt)
	return rl.Readline()
}

func stringOrDefault(s *string, def string) string {
	if s != nil && *s != "" {
		return *s
	}
	return def
}
