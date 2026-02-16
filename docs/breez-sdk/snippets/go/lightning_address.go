package example

import (
	"errors"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ConfigLightningAddress() *breez_sdk_spark.Config {
	// ANCHOR: config-lightning-address
	lnurlDomain := "yourdomain.com"
	apiKey := "your-api-key"
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	config.ApiKey = &apiKey
	config.LnurlDomain = &lnurlDomain
	// ANCHOR_END: config-lightning-address
	return &config
}

func CheckLightningAddressAvailability(client *breez_sdk_spark.BreezClient) (bool, error) {
	username := "myusername"

	// ANCHOR: check-lightning-address
	request := breez_sdk_spark.CheckLightningAddressRequest{
		Username: username,
	}

	isAvailable, err := client.CheckLightningAddressAvailable(request)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return false, err
	}
	// ANCHOR_END: check-lightning-address

	return isAvailable, nil
}

func RegisterLightningAddress(client *breez_sdk_spark.BreezClient) (*breez_sdk_spark.LightningAddressInfo, error) {
	username := "myusername"
	description := "My Lightning Address"

	// ANCHOR: register-lightning-address
	request := breez_sdk_spark.RegisterLightningAddressRequest{
		Username:    username,
		Description: &description,
	}

	addressInfo, err := client.RegisterLightningAddress(request)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	_ = addressInfo.LightningAddress
	_ = addressInfo.Lnurl.Url
	_ = addressInfo.Lnurl.Bech32
	// ANCHOR_END: register-lightning-address

	return &addressInfo, nil
}

func GetLightningAddress(client *breez_sdk_spark.BreezClient) (*breez_sdk_spark.LightningAddressInfo, error) {
	// ANCHOR: get-lightning-address
	addressInfoOpt, err := client.GetLightningAddress()
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	if addressInfoOpt != nil {
		_ = addressInfoOpt.LightningAddress
		_ = addressInfoOpt.Username
		_ = addressInfoOpt.Description
		_ = addressInfoOpt.Lnurl.Url
		_ = addressInfoOpt.Lnurl.Bech32
	}
	// ANCHOR_END: get-lightning-address

	return addressInfoOpt, nil
}

func DeleteLightningAddress(client *breez_sdk_spark.BreezClient) error {
	// ANCHOR: delete-lightning-address
	err := client.DeleteLightningAddress()
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}
	// ANCHOR_END: delete-lightning-address

	return nil
}

func AccessSenderComment(client *breez_sdk_spark.BreezClient) error {
	paymentID := "<payment id>"
	response, err := client.GetPayment(breez_sdk_spark.GetPaymentRequest{
		PaymentId: paymentID,
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}
	payment := response.Payment

	// ANCHOR: access-sender-comment
	// Check if this is a lightning payment with LNURL receive metadata
	if lightningDetails, ok := (*payment.Details).(breez_sdk_spark.PaymentDetailsLightning); ok {
		metadata := lightningDetails.LnurlReceiveMetadata

		// Access the sender comment if present
		if metadata != nil && metadata.SenderComment != nil {
			println("Sender comment:", *metadata.SenderComment)
		}
	}
	// ANCHOR_END: access-sender-comment
	return nil
}

func AccessNostrZap(client *breez_sdk_spark.BreezClient) error {
	paymentID := "<payment id>"
	response, err := client.GetPayment(breez_sdk_spark.GetPaymentRequest{
		PaymentId: paymentID,
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}
	payment := response.Payment

	// ANCHOR: access-nostr-zap
	// Check if this is a lightning payment with LNURL receive metadata
	if lightningDetails, ok := (*payment.Details).(breez_sdk_spark.PaymentDetailsLightning); ok {
		metadata := lightningDetails.LnurlReceiveMetadata

		if metadata != nil {
			// Access the Nostr zap request if present
			if metadata.NostrZapRequest != nil {
				// The NostrZapRequest is a JSON string containing the Nostr event (kind 9734)
				println("Nostr zap request:", *metadata.NostrZapRequest)
			}

			// Access the Nostr zap receipt if present
			if metadata.NostrZapReceipt != nil {
				// The NostrZapReceipt is a JSON string containing the Nostr event (kind 9735)
				println("Nostr zap receipt:", *metadata.NostrZapReceipt)
			}
		}
	}
	// ANCHOR_END: access-nostr-zap
	return nil
}
