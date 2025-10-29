package example

import (
	"log"
	"strconv"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_common"
	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ParseInput(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_common.InputType, error) {
	// ANCHOR: parse-inputs
	inputStr := "an input to be parsed..."

	input, err := sdk.Parse(inputStr)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	switch inputType := input.(type) {
	case breez_sdk_common.InputTypeBitcoinAddress:
		log.Printf("Input is Bitcoin address %s", inputType.Field0.Address)

	case breez_sdk_common.InputTypeBolt11Invoice:
		amount := "unknown"
		if inputType.Field0.AmountMsat != nil {
			amount = strconv.FormatUint(*inputType.Field0.AmountMsat, 10)
		}
		log.Printf("Input is BOLT11 invoice for %s msats", amount)

	case breez_sdk_common.InputTypeLnurlPay:
		log.Printf("Input is LNURL-Pay/Lightning address accepting min/max %d/%d msats",
			inputType.Field0.MinSendable, inputType.Field0.MaxSendable)

	case breez_sdk_common.InputTypeLnurlWithdraw:
		log.Printf("Input is LNURL-Withdraw for min/max %d/%d msats",
			inputType.Field0.MinWithdrawable, inputType.Field0.MaxWithdrawable)

	case breez_sdk_common.InputTypeSparkAddress:
		log.Printf("Input is Spark address %s", inputType.Field0.Address)

	case breez_sdk_common.InputTypeSparkInvoice:
		invoice := inputType.Field0
		log.Println("Input is Spark invoice:")
		if invoice.TokenIdentifier != nil {
			log.Printf("  Amount: %d base units of token with id %s", invoice.Amount, *invoice.TokenIdentifier)
		} else {
			log.Printf("  Amount: %d sats", invoice.Amount)
		}

		if invoice.Description != nil {
			log.Printf("  Description: %s", *invoice.Description)
		}

		if invoice.ExpiryTime != nil {
			log.Printf("  Expiry time: %d", *invoice.ExpiryTime)
		}

		if invoice.SenderPublicKey != nil {
			log.Printf("  Sender public key: %s", *invoice.SenderPublicKey)
		}

	default:
		// Other input types are available
	}
	// ANCHOR_END: parse-inputs
	return &input, nil
}

func SetExternalInputParsers() (*breez_sdk_spark.Config, error) {
	// ANCHOR: set-external-input-parsers
	// Create the default config
	apiKey := "<breez api key>"
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.ApiKey = &apiKey

	// Configure external parsers
	parsers := []breez_sdk_common.ExternalInputParser{
		{
			ProviderId: "provider_a",
			InputRegex: "^provider_a",
			ParserUrl:  "https://parser-domain.com/parser?input=<input>",
		},
		{
			ProviderId: "provider_b",
			InputRegex: "^provider_b",
			ParserUrl:  "https://parser-domain.com/parser?input=<input>",
		},
	}
	config.ExternalInputParsers = &parsers
	// ANCHOR_END: set-external-input-parsers
	return &config, nil
}
