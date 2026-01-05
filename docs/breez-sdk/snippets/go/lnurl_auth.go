package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func parseLnurlAuth(sdk *breez_sdk_spark.BreezSdk) {
	// ANCHOR: parse-lnurl-auth
	// LNURL-auth URL from a service
	// Can be in the form:
	// - lnurl1... (bech32 encoded)
	// - https://service.com/lnurl-auth?tag=login&k1=...
	lnurlAuthUrl := "lnurl1..."

	inputType, err := sdk.Parse(lnurlAuthUrl)
	if err == nil {
		if lnurlAuth, ok := inputType.(breez_sdk_spark.InputTypeLnurlAuth); ok {
			requestData := lnurlAuth.Field0
			log.Printf("Domain: %s", requestData.Domain)
			log.Printf("Action: %v", requestData.Action)

			// Show domain to user and ask for confirmation
			// This is important for security
		}
	}
	// ANCHOR_END: parse-lnurl-auth
}

func authenticate(sdk *breez_sdk_spark.BreezSdk, requestData breez_sdk_spark.LnurlAuthRequestDetails) {
	// ANCHOR: lnurl-auth
	// Perform LNURL authentication
	result, err := sdk.LnurlAuth(requestData)
	if err != nil {
		log.Printf("Authentication error: %v", err)
		return
	}

	switch v := result.(type) {
	case breez_sdk_spark.LnurlCallbackStatusOk:
		log.Println("Authentication successful")
	case breez_sdk_spark.LnurlCallbackStatusErrorStatus:
		log.Printf("Authentication failed: %s", v.ErrorDetails.Reason)
	}
	// ANCHOR_END: lnurl-auth
}
