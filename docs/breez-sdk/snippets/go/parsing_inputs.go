package example

import (
	"log"
	"strconv"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_common"
	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ParseInput() (*breez_sdk_common.InputType, error) {
	// ANCHOR: parse-inputs
	inputStr := "an input to be parsed..."

	input, err := breez_sdk_spark.Parse(inputStr)

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

	default:
		// Other input types are available
	}
	// ANCHOR_END: parse-inputs
	return &input, nil
}
