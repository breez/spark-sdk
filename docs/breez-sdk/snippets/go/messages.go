package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func SignMessage(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.SignMessageResponse, error) {
	// ANCHOR: sign-message
	message := "<message to sign>"
	// Set to true to get a compact signature rather than a DER
	compact := true

	signMessageRequest := breez_sdk_spark.SignMessageRequest{
		Message: message,
		Compact: compact,
	}
	signMessageResponse, err := sdk.SignMessage(signMessageRequest)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	signature := signMessageResponse.Signature
	pubkey := signMessageResponse.Pubkey

	log.Printf("Pubkey: %v", pubkey)
	log.Printf("Signature: %v", signature)
	// ANCHOR_END: sign-message
	return &signMessageResponse, nil
}

func CheckMessage(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.CheckMessageResponse, error) {
	// ANCHOR: check-message
	message := "<message>"
	pubkey := "<pubkey of signer>"
	signature := "<message signature>"

	checkMessageRequest := breez_sdk_spark.CheckMessageRequest{
		Message:   message,
		Pubkey:    pubkey,
		Signature: signature,
	}
	checkMessageResponse, err := sdk.CheckMessage(checkMessageRequest)
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	isValid := checkMessageResponse.IsValid

	log.Printf("Signature valid: %v", isValid)
	// ANCHOR_END: check-message
	return &checkMessageResponse, nil
}
