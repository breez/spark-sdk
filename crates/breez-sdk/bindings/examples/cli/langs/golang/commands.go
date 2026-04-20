package main

import (
	cryptorand "crypto/rand"
	"crypto/sha256"
	"encoding/hex"
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
	fmt.Println("\nAvailable commands:")
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
	fmt.Printf("  %-40s %s\n", "contacts <subcommand>", "Contacts commands (use 'contacts help' for details)")
	fmt.Printf("  %-40s %s\n", "exit / quit", "Exit the CLI")
	fmt.Printf("  %-40s %s\n", "help", "Show this help message")
	fmt.Println()
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

// --- get-info ---

func handleGetInfo(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("get-info", flag.ContinueOnError)
	ensureSynced := fs.String("s", "", "Force sync (true/false)")
	fs.StringVar(ensureSynced, "ensure-synced", "", "Force sync (true/false)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	req := breez_sdk_spark.GetInfoRequest{}
	if *ensureSynced != "" {
		val := *ensureSynced == "true"
		req.EnsureSynced = &val
	}

	result, err := sdk.GetInfo(req)
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
	var typeFilter stringSliceFlag
	fs.Var(&typeFilter, "t", "Filter by payment type (send, receive)")
	fs.Var(&typeFilter, "type-filter", "Filter by payment type")
	var statusFilter stringSliceFlag
	fs.Var(&statusFilter, "s", "Filter by payment status (pending, completed, failed)")
	fs.Var(&statusFilter, "status-filter", "Filter by payment status")
	assetFilterStr := fs.String("a", "", "Filter by asset (bitcoin, token:<id>)")
	fs.StringVar(assetFilterStr, "asset-filter", "", "Filter by asset")
	var sparkHtlcStatusFilter stringSliceFlag
	fs.Var(&sparkHtlcStatusFilter, "spark-htlc-status-filter", "Filter by Spark HTLC status")
	txHash := fs.String("tx-hash", "", "Filter by token transaction hash")
	txType := fs.String("tx-type", "", "Filter by token transaction type")
	fromTimestamp := fs.Uint64("from-timestamp", 0, "Only include payments created after this timestamp")
	toTimestamp := fs.Uint64("to-timestamp", 0, "Only include payments created before this timestamp")
	sortAscending := fs.String("sort-ascending", "", "Sort payments in ascending order (true/false)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	limitU32 := uint32(*limit)
	offsetU32 := uint32(*offset)

	req := breez_sdk_spark.ListPaymentsRequest{
		Limit:  &limitU32,
		Offset: &offsetU32,
	}

	if len(typeFilter) > 0 {
		types := parsePaymentTypes(typeFilter)
		req.TypeFilter = &types
	}

	if len(statusFilter) > 0 {
		statuses := parsePaymentStatuses(statusFilter)
		req.StatusFilter = &statuses
	}

	if *assetFilterStr != "" {
		af := parseAssetFilter(*assetFilterStr)
		if af != nil {
			req.AssetFilter = &af
		}
	}

	var paymentDetailsFilter []breez_sdk_spark.PaymentDetailsFilter
	if len(sparkHtlcStatusFilter) > 0 {
		htlcStatuses := parseSparkHtlcStatuses(sparkHtlcStatusFilter)
		paymentDetailsFilter = append(paymentDetailsFilter, breez_sdk_spark.PaymentDetailsFilterSpark{
			HtlcStatus: &htlcStatuses,
		})
	}
	if *txHash != "" {
		paymentDetailsFilter = append(paymentDetailsFilter, breez_sdk_spark.PaymentDetailsFilterToken{
			TxHash: txHash,
		})
	}
	if *txType != "" {
		tt := parseTokenTransactionType(*txType)
		if tt != nil {
			paymentDetailsFilter = append(paymentDetailsFilter, breez_sdk_spark.PaymentDetailsFilterToken{
				TxType: tt,
			})
		}
	}
	if len(paymentDetailsFilter) > 0 {
		req.PaymentDetailsFilter = &paymentDetailsFilter
	}

	if *fromTimestamp > 0 {
		req.FromTimestamp = fromTimestamp
	}
	if *toTimestamp > 0 {
		req.ToTimestamp = toTimestamp
	}
	if *sortAscending != "" {
		val := *sortAscending == "true"
		req.SortAscending = &val
	}

	result, err := sdk.ListPayments(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- receive ---

func handleReceive(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("receive", flag.ContinueOnError)
	method := fs.String("m", "", "Payment method: sparkaddress, sparkinvoice, bitcoin, bolt11")
	fs.StringVar(method, "method", "", "Payment method")
	description := fs.String("d", "", "Optional description")
	fs.StringVar(description, "description", "", "Optional description")
	amountStr := fs.String("a", "", "Amount in sats or token base units")
	fs.StringVar(amountStr, "amount", "", "Amount")
	tokenId := fs.String("t", "", "Optional token identifier (sparkinvoice only)")
	fs.StringVar(tokenId, "token-identifier", "", "Optional token identifier")
	expirySecs := fs.Uint("e", 0, "Optional expiry time in seconds from now")
	fs.UintVar(expirySecs, "expiry-secs", 0, "Optional expiry time in seconds")
	senderPublicKey := fs.String("s", "", "Optional sender public key (sparkinvoice only)")
	fs.StringVar(senderPublicKey, "sender-public-key", "", "Optional sender public key")
	hodl := fs.Bool("hodl", false, "Create a HODL invoice (bolt11 only)")
	newAddress := fs.Bool("new-address", false, "Get a new bitcoin deposit address (bitcoin only)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *method == "" {
		fmt.Println("Usage: receive -m <method> [options]")
		fmt.Println("Methods: sparkaddress, sparkinvoice, bitcoin, bolt11")
		return nil
	}

	var paymentMethod breez_sdk_spark.ReceivePaymentMethod

	switch strings.ToLower(*method) {
	case "sparkaddress":
		paymentMethod = breez_sdk_spark.ReceivePaymentMethodSparkAddress{}

	case "sparkinvoice":
		pm := breez_sdk_spark.ReceivePaymentMethodSparkInvoice{}
		if *amountStr != "" {
			amount, ok := new(big.Int).SetString(*amountStr, 10)
			if !ok {
				return fmt.Errorf("invalid amount: %s", *amountStr)
			}
			pm.Amount = &amount
		}
		if *tokenId != "" {
			pm.TokenIdentifier = tokenId
		}
		if *expirySecs > 0 {
			now := uint64(time.Now().Unix())
			expiryTime := now + uint64(*expirySecs)
			pm.ExpiryTime = &expiryTime
		}
		if *description != "" {
			pm.Description = description
		}
		if *senderPublicKey != "" {
			pm.SenderPublicKey = senderPublicKey
		}
		paymentMethod = pm

	case "bitcoin":
		paymentMethod = breez_sdk_spark.ReceivePaymentMethodBitcoinAddress{
			NewAddress: newAddress,
		}

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
		if *expirySecs > 0 {
			e := uint32(*expirySecs)
			pm.ExpirySecs = &e
		}
		if *hodl {
			preimageBytes := make([]byte, 32)
			if _, err := cryptorand.Read(preimageBytes); err != nil {
				return fmt.Errorf("failed to generate preimage: %w", err)
			}
			preimage := hex.EncodeToString(preimageBytes)
			hashBytes := sha256.Sum256(preimageBytes)
			paymentHash := hex.EncodeToString(hashBytes[:])

			fmt.Printf("HODL invoice preimage: %s\n", preimage)
			fmt.Printf("Payment hash: %s\n", paymentHash)
			fmt.Println("Save the preimage! Use `claim-htlc-payment` with it to settle.")

			pm.PaymentHash = &paymentHash
		}
		paymentMethod = pm

	default:
		fmt.Printf("Invalid payment method: %s\n", *method)
		fmt.Println("Available methods: sparkaddress, sparkinvoice, bitcoin, bolt11")
		return nil
	}

	result, err := sdk.ReceivePayment(breez_sdk_spark.ReceivePaymentRequest{
		PaymentMethod: paymentMethod,
	})
	if err = liftError(err); err != nil {
		return err
	}

	if result.Fee != nil && result.Fee.Sign() > 0 {
		fmt.Printf("Prepared payment requires fee of %s sats/token base units\n", result.Fee.String())
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
	idempotencyKey := fs.String("i", "", "Optional idempotency key")
	fs.StringVar(idempotencyKey, "idempotency-key", "", "Optional idempotency key")
	fromBitcoin := fs.Bool("from-bitcoin", false, "Convert from Bitcoin to token to fulfill payment")
	fromToken := fs.String("from-token", "", "Convert from token to Bitcoin to fulfill payment")
	maxSlippage := fs.String("s", "", "Max slippage in basis points for conversion")
	fs.StringVar(maxSlippage, "convert-max-slippage-bps", "", "Max slippage in basis points")
	feesIncluded := fs.Bool("fees-included", false, "Deduct fees from amount instead of adding on top")
	if err := fs.Parse(args); err != nil {
		return err
	}

	if *paymentRequest == "" {
		fmt.Println("Usage: pay -r <payment_request> [-a <amount>] [-t <token_identifier>] [--from-bitcoin | --from-token <id>] [-s <slippage_bps>] [--fees-included] [-i <idempotency_key>]")
		return nil
	}

	req := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest: breez_sdk_spark.PaymentRequestInput{Input: *paymentRequest},
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

	// Conversion options
	if *fromBitcoin || *fromToken != "" {
		var maxSlippageBps *uint32
		if *maxSlippage != "" {
			val, err := strconv.ParseUint(*maxSlippage, 10, 32)
			if err != nil {
				return fmt.Errorf("invalid max slippage: %s", *maxSlippage)
			}
			val32 := uint32(val)
			maxSlippageBps = &val32
		}

		var convType breez_sdk_spark.ConversionType
		if *fromBitcoin {
			convType = breez_sdk_spark.ConversionTypeFromBitcoin{}
		} else {
			convType = breez_sdk_spark.ConversionTypeToBitcoin{FromTokenIdentifier: *fromToken}
		}
		convOpts := breez_sdk_spark.ConversionOptions{
			ConversionType: convType,
			MaxSlippageBps: maxSlippageBps,
		}
		req.ConversionOptions = &convOpts
	}

	// Fee policy
	if *feesIncluded {
		fp := breez_sdk_spark.FeePolicyFeesIncluded
		req.FeePolicy = &fp
	}

	prepareResponse, err := sdk.PrepareSendPayment(req)
	if err = liftError(err); err != nil {
		return fmt.Errorf("failed to prepare payment: %w", err)
	}

	// Show conversion estimate and confirm
	if prepareResponse.ConversionEstimate != nil {
		est := prepareResponse.ConversionEstimate
		units := "token base units"
		if _, ok := est.Options.ConversionType.(breez_sdk_spark.ConversionTypeFromBitcoin); ok {
			units = "sats"
		}
		fmt.Printf("Estimated conversion of %v %s → %v %s with a %v %s fee\n", est.AmountIn, units, est.AmountOut, units, est.Fee, units)
		line, err := readlineWithDefault(rl, "Do you want to continue (y/n): ", "y")
		if err != nil {
			return err
		}
		if strings.ToLower(strings.TrimSpace(line)) != "y" {
			return fmt.Errorf("payment cancelled")
		}
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
	if *idempotencyKey != "" {
		sendReq.IdempotencyKey = idempotencyKey
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
	idempotencyKey := fs.String("i", "", "Optional idempotency key")
	fs.StringVar(idempotencyKey, "idempotency-key", "", "Optional idempotency key")
	fromToken := fs.String("from-token", "", "Convert from token to Bitcoin to fulfill payment")
	maxSlippage := fs.String("s", "", "Max slippage in basis points for conversion")
	fs.StringVar(maxSlippage, "convert-max-slippage-bps", "", "Max slippage in basis points")
	feesIncluded := fs.Bool("fees-included", false, "Deduct fees from amount instead of adding on top")
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

	minSendable := (payRequest.MinSendable + 999) / 1000
	maxSendable := payRequest.MaxSendable / 1000
	prompt := fmt.Sprintf("Amount to pay (min %d sat, max %d sat): ", minSendable, maxSendable)
	amountLine, err := readlinePrompt(rl, prompt)
	if err != nil {
		return err
	}
	amountSats, err := strconv.ParseUint(strings.TrimSpace(amountLine), 10, 64)
	if err != nil {
		return fmt.Errorf("invalid amount: %w", err)
	}

	prepareReq := breez_sdk_spark.PrepareLnurlPayRequest{
		Amount: new(big.Int).SetUint64(amountSats),
		PayRequest: payRequest,
	}
	if *comment != "" {
		prepareReq.Comment = comment
	}
	if *validateStr != "" {
		val := *validateStr == "true"
		prepareReq.ValidateSuccessActionUrl = &val
	}

	// Conversion options
	if *fromToken != "" {
		var maxSlippageBps *uint32
		if *maxSlippage != "" {
			val, err := strconv.ParseUint(*maxSlippage, 10, 32)
			if err != nil {
				return fmt.Errorf("invalid max slippage: %s", *maxSlippage)
			}
			val32 := uint32(val)
			maxSlippageBps = &val32
		}
		convOpts := breez_sdk_spark.ConversionOptions{
			ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{FromTokenIdentifier: *fromToken},
			MaxSlippageBps: maxSlippageBps,
		}
		prepareReq.ConversionOptions = &convOpts
	}

	// Fee policy
	if *feesIncluded {
		fp := breez_sdk_spark.FeePolicyFeesIncluded
		prepareReq.FeePolicy = &fp
	}

	prepareResponse, err := sdk.PrepareLnurlPay(prepareReq)
	if err = liftError(err); err != nil {
		return err
	}

	// Show conversion estimate and confirm
	if prepareResponse.ConversionEstimate != nil {
		est := prepareResponse.ConversionEstimate
		fmt.Printf("Estimated conversion of %v token base units → %v sats with a %v token base units fee\n", est.AmountIn, est.AmountOut, est.Fee)
		line, err := readlineWithDefault(rl, "Do you want to continue (y/n): ", "y")
		if err != nil {
			return err
		}
		if strings.ToLower(strings.TrimSpace(line)) != "y" {
			return fmt.Errorf("payment cancelled")
		}
	}

	printValue(prepareResponse)

	line, err := readlineWithDefault(rl, "Do you want to continue? (y/n): ", "y")
	if err != nil {
		return err
	}
	if strings.ToLower(strings.TrimSpace(line)) != "y" {
		return nil
	}

	lnurlPayReq := breez_sdk_spark.LnurlPayRequest{
		PrepareResponse: prepareResponse,
	}
	if *idempotencyKey != "" {
		lnurlPayReq.IdempotencyKey = idempotencyKey
	}

	result, err := sdk.LnurlPay(lnurlPayReq)
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

	minWithdrawable := (withdrawData.Field0.MinWithdrawable + 999) / 1000
	maxWithdrawable := withdrawData.Field0.MaxWithdrawable / 1000
	prompt := fmt.Sprintf("Amount to withdraw (min %d sat, max %d sat): ", minWithdrawable, maxWithdrawable)
	amountLine, err := readlinePrompt(rl, prompt)
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

func handleLnurlAuth(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error {
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

	action := "auth"
	if authData.Field0.Action != nil {
		action = *authData.Field0.Action
	}
	prompt := fmt.Sprintf("Authenticate with %s (action: %s)? (y/n): ", authData.Field0.Domain, action)
	line, err := readlineWithDefault(rl, prompt, "y")
	if err != nil {
		return err
	}
	if strings.ToLower(strings.TrimSpace(line)) != "y" {
		return nil
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
	printValue(result.Payment)
	return nil
}

// --- claim-deposit ---

func handleClaimDeposit(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("claim-deposit", flag.ContinueOnError)
	feeSat := fs.Uint64("fee-sat", 0, "Max fee in sats (fixed)")
	satPerVbyte := fs.Uint64("sat-per-vbyte", 0, "Max fee per vbyte (rate)")
	recommendedFeeLeeway := fs.Uint64("recommended-fee-leeway", 0, "Use fastest recommended fee plus this leeway (sat/vbyte)")
	if err := fs.Parse(args); err != nil {
		return err
	}

	positional := fs.Args()
	if len(positional) < 2 {
		fmt.Println("Usage: claim-deposit <txid> <vout> [--fee-sat N | --sat-per-vbyte N | --recommended-fee-leeway N]")
		return nil
	}

	txid := positional[0]
	vout, err := strconv.ParseUint(positional[1], 10, 32)
	if err != nil {
		return fmt.Errorf("invalid vout: %w", err)
	}

	var maxFee breez_sdk_spark.MaxFee
	if *recommendedFeeLeeway > 0 {
		if *feeSat > 0 || *satPerVbyte > 0 {
			return fmt.Errorf("cannot specify fee-sat or sat-per-vbyte when using recommended fee")
		}
		maxFee = breez_sdk_spark.MaxFeeNetworkRecommended{LeewaySatPerVbyte: *recommendedFeeLeeway}
	} else if *feeSat > 0 && *satPerVbyte > 0 {
		return fmt.Errorf("cannot specify both --fee-sat and --sat-per-vbyte")
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

	req := breez_sdk_spark.BuyBitcoinRequestMoonpay{}
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
	fmt.Println("Open this URL in a browser to complete the purchase:")
	fmt.Println(result.Url)
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
		completionTimeout := uint32(0)
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
					PreferSpark:           true,
					CompletionTimeoutSecs: &completionTimeout,
				}
				return &opts, nil
			}
		}
		var opts breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
			PreferSpark:           false,
			CompletionTimeoutSecs: &completionTimeout,
		}
		return &opts, nil

	case breez_sdk_spark.SendPaymentMethodSparkAddress:
		// HTLC options are only valid for Bitcoin payments, not token payments
		if pm.TokenIdentifier != nil {
			return nil, nil
		}

		line, err := readlineWithDefault(rl, "Do you want to create an HTLC transfer? (y/n): ", "n")
		if err != nil {
			return nil, err
		}
		if strings.ToLower(strings.TrimSpace(line)) != "y" {
			return nil, nil
		}

		paymentHashLine, err := readlinePrompt(rl, "Please enter the HTLC payment hash (hex string) or leave empty to generate a new preimage and associated hash: ")
		if err != nil {
			return nil, err
		}
		paymentHash := strings.TrimSpace(paymentHashLine)
		if paymentHash == "" {
			preimageBytes := make([]byte, 32)
			if _, err := cryptorand.Read(preimageBytes); err != nil {
				return nil, fmt.Errorf("failed to generate preimage: %w", err)
			}
			preimage := hex.EncodeToString(preimageBytes)
			hashBytes := sha256.Sum256(preimageBytes)
			paymentHash = hex.EncodeToString(hashBytes[:])

			fmt.Printf("Generated preimage: %s\n", preimage)
			fmt.Printf("Associated payment hash: %s\n", paymentHash)
		}

		expiryLine, err := readlinePrompt(rl, "Please enter the HTLC expiry duration in seconds: ")
		if err != nil {
			return nil, err
		}
		expiryDurationSecs, err := strconv.ParseUint(strings.TrimSpace(expiryLine), 10, 64)
		if err != nil {
			return nil, fmt.Errorf("invalid expiry duration: %w", err)
		}

		htlcOptions := breez_sdk_spark.SparkHtlcOptions{
			PaymentHash:        paymentHash,
			ExpiryDurationSecs: expiryDurationSecs,
		}
		var opts breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsSparkAddress{
			HtlcOptions: &htlcOptions,
		}
		return &opts, nil
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

// stringSliceFlag implements flag.Value for a comma-separated list of strings.
type stringSliceFlag []string

func (s *stringSliceFlag) String() string {
	return strings.Join(*s, ",")
}

func (s *stringSliceFlag) Set(value string) error {
	for _, v := range strings.Split(value, ",") {
		v = strings.TrimSpace(v)
		if v != "" {
			*s = append(*s, v)
		}
	}
	return nil
}

func parsePaymentTypes(values []string) []breez_sdk_spark.PaymentType {
	var result []breez_sdk_spark.PaymentType
	for _, v := range values {
		switch strings.ToLower(v) {
		case "send":
			result = append(result, breez_sdk_spark.PaymentTypeSend)
		case "receive":
			result = append(result, breez_sdk_spark.PaymentTypeReceive)
		}
	}
	return result
}

func parsePaymentStatuses(values []string) []breez_sdk_spark.PaymentStatus {
	var result []breez_sdk_spark.PaymentStatus
	for _, v := range values {
		switch strings.ToLower(v) {
		case "pending":
			result = append(result, breez_sdk_spark.PaymentStatusPending)
		case "completed":
			result = append(result, breez_sdk_spark.PaymentStatusCompleted)
		case "failed":
			result = append(result, breez_sdk_spark.PaymentStatusFailed)
		}
	}
	return result
}

func parseAssetFilter(value string) breez_sdk_spark.AssetFilter {
	lower := strings.ToLower(value)
	if lower == "bitcoin" {
		return breez_sdk_spark.AssetFilterBitcoin{}
	}
	if strings.HasPrefix(lower, "token:") {
		tokenId := value[6:]
		return breez_sdk_spark.AssetFilterToken{TokenIdentifier: &tokenId}
	}
	if lower == "token" {
		return breez_sdk_spark.AssetFilterToken{}
	}
	return nil
}

func parseSparkHtlcStatuses(values []string) []breez_sdk_spark.SparkHtlcStatus {
	var result []breez_sdk_spark.SparkHtlcStatus
	for _, v := range values {
		switch strings.ToLower(v) {
		case "waitingforpreimage":
			result = append(result, breez_sdk_spark.SparkHtlcStatusWaitingForPreimage)
		case "preimageshared":
			result = append(result, breez_sdk_spark.SparkHtlcStatusPreimageShared)
		case "returned":
			result = append(result, breez_sdk_spark.SparkHtlcStatusReturned)
		}
	}
	return result
}

func parseTokenTransactionType(value string) *breez_sdk_spark.TokenTransactionType {
	switch strings.ToLower(value) {
	case "mint":
		t := breez_sdk_spark.TokenTransactionType(breez_sdk_spark.TokenTransactionTypeMint)
		return &t
	case "burn":
		t := breez_sdk_spark.TokenTransactionType(breez_sdk_spark.TokenTransactionTypeBurn)
		return &t
	case "transfer":
		t := breez_sdk_spark.TokenTransactionType(breez_sdk_spark.TokenTransactionTypeTransfer)
		return &t
	}
	return nil
}
