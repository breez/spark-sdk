package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetX402(sdk *breez_sdk_spark.BreezSdk) *breez_sdk_spark.X402 {
	// ANCHOR: get-x402
	x402 := sdk.GetX402()
	// ANCHOR_END: get-x402
	return x402
}

func PayHttpRequest(x402 *breez_sdk_spark.X402, paymentRequiredHeader string) error {
	// ANCHOR: pay-http-request
	// The request that returned 402, exactly as it will be sent again
	var request breez_sdk_spark.X402Request = breez_sdk_spark.X402RequestHttp{
		Method: "GET",
		Url:    "https://api.example.com/article/A",
		Headers: []breez_sdk_spark.X402Header{
			{Name: "accept", Value: "application/json"},
		},
		Body: nil,
	}
	response, err := x402.Pay(breez_sdk_spark.PayX402Request{
		// The PAYMENT-REQUIRED header of the 402 response
		PaymentRequired:       paymentRequiredHeader,
		Request:               request,
		MaxAmount:             breez_sdk_spark.X402PaymentAmountLightning{AmountSats: 100},
		ConversionOptions:     nil,
		CompletionTimeoutSecs: nil,
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
		}
		return err
	}

	if response.PaymentSignature != nil {
		// Send the request again with a PAYMENT-SIGNATURE header
		log.Printf("PAYMENT-SIGNATURE: %v", *response.PaymentSignature)
	} else {
		// Still in flight: paying the same challenge again picks it up
		log.Printf("Payment status: %v", response.Status)
	}
	// ANCHOR_END: pay-http-request
	return nil
}

func PayMcpToolCall(x402 *breez_sdk_spark.X402, structuredContent string) error {
	// ANCHOR: pay-mcp-tool-call
	var request breez_sdk_spark.X402Request = breez_sdk_spark.X402RequestMcp{
		Server: "https://api.example.com/mcp",
		// The tools/call params, exactly as they will be sent again
		Params: `{"name":"get_article","arguments":{"article":"A"}}`,
	}
	response, err := x402.Pay(breez_sdk_spark.PayX402Request{
		// The structuredContent of the tool result
		PaymentRequired:       structuredContent,
		Request:               request,
		MaxAmount:             breez_sdk_spark.X402PaymentAmountLightning{AmountSats: 100},
		ConversionOptions:     nil,
		CompletionTimeoutSecs: nil,
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
		}
		return err
	}

	if response.PaymentSignature != nil {
		// Call the tool again with this JSON in params._meta["x402/payment"]
		log.Printf("Payment: %v", *response.PaymentSignature)
	}
	// ANCHOR_END: pay-mcp-tool-call
	return nil
}

func CreateChallenge(x402 *breez_sdk_spark.X402) error {
	// ANCHOR: create-challenge
	// The unpaid request, with its URL built from your public origin
	var request breez_sdk_spark.X402Request = breez_sdk_spark.X402RequestHttp{
		Method: "GET",
		Url:    "https://api.example.com/article/A",
		Headers: []breez_sdk_spark.X402Header{
			{Name: "accept", Value: "application/json"},
		},
		Body: nil,
	}
	optionalDescription := "An article"
	optionalMimeType := "application/json"
	response, err := x402.CreateChallenge(breez_sdk_spark.CreateX402ChallengeRequest{
		Request: request,
		// Headers that change what is served, so the payment covers them too
		BoundNames: []string{"accept"},
		Amounts: []breez_sdk_spark.X402PaymentAmount{
			breez_sdk_spark.X402PaymentAmountLightning{AmountSats: 21},
		},
		MaxTimeoutSecs: nil,
		Description:    &optionalDescription,
		MimeType:       &optionalMimeType,
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
		}
		return err
	}

	// Respond with status 402 and this PAYMENT-REQUIRED header
	log.Printf("PAYMENT-REQUIRED: %v", response.PaymentRequired)
	// ANCHOR_END: create-challenge
	return nil
}

func SettlePayment(x402 *breez_sdk_spark.X402, paymentSignatureHeader string) error {
	// ANCHOR: settle-payment
	// The paid request, exactly as received
	var request breez_sdk_spark.X402Request = breez_sdk_spark.X402RequestHttp{
		Method: "GET",
		Url:    "https://api.example.com/article/A",
		Headers: []breez_sdk_spark.X402Header{
			{Name: "accept", Value: "application/json"},
		},
		Body: nil,
	}
	response, err := x402.Settle(breez_sdk_spark.SettleX402Request{
		// The PAYMENT-SIGNATURE header of the request
		Payment:    paymentSignatureHeader,
		Request:    request,
		BoundNames: []string{"accept"},
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
		}
		return err
	}

	// Send the PAYMENT-RESPONSE header either way
	log.Printf("PAYMENT-RESPONSE: %v", response.PaymentResponse)
	if response.Success {
		// Serve the resource
	} else {
		// Respond with status 402 and a new challenge
		if response.ErrorReason != nil {
			log.Printf("Payment refused: %v", *response.ErrorReason)
		}
	}
	// ANCHOR_END: settle-payment
	return nil
}
