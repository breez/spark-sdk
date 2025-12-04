package example

import (
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

func CheckLightningAddressAvailability(sdk *breez_sdk_spark.BreezSdk) (bool, error) {
	username := "myusername"

	// ANCHOR: check-lightning-address
	request := breez_sdk_spark.CheckLightningAddressRequest{
		Username: username,
	}

	isAvailable, err := sdk.CheckLightningAddressAvailable(request)
	if err != nil {
		return false, err
	}
	// ANCHOR_END: check-lightning-address

	return isAvailable, nil
}

func RegisterLightningAddress(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.LightningAddressInfo, error) {
	username := "myusername"
	description := "My Lightning Address"

	// ANCHOR: register-lightning-address
	request := breez_sdk_spark.RegisterLightningAddressRequest{
		Username:    username,
		Description: &description,
	}

	addressInfo, err := sdk.RegisterLightningAddress(request)
	if err != nil {
		return nil, err
	}

	_ = addressInfo.LightningAddress
	_ = addressInfo.Lnurl
	// ANCHOR_END: register-lightning-address

	return &addressInfo, nil
}

func GetLightningAddress(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.LightningAddressInfo, error) {
	// ANCHOR: get-lightning-address
	addressInfoOpt, err := sdk.GetLightningAddress()
	if err != nil {
		return nil, err
	}

	if addressInfoOpt != nil {
		_ = addressInfoOpt.LightningAddress
		_ = addressInfoOpt.Username
		_ = addressInfoOpt.Description
		_ = addressInfoOpt.Lnurl
	}
	// ANCHOR_END: get-lightning-address

	return addressInfoOpt, nil
}

func DeleteLightningAddress(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: delete-lightning-address
	err := sdk.DeleteLightningAddress()
	if err != nil {
		return err
	}
	// ANCHOR_END: delete-lightning-address

	return nil
}

func AccessSenderComment(sdk *breez_sdk_spark.BreezSdk) error {
	paymentID := "<payment id>"
	response, err := sdk.GetPayment(breez_sdk_spark.GetPaymentRequest{
		PaymentId: paymentID,
	})
	if err != nil {
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

func AccessNostrZap(sdk *breez_sdk_spark.BreezSdk) error {
	paymentID := "<payment id>"
	response, err := sdk.GetPayment(breez_sdk_spark.GetPaymentRequest{
		PaymentId: paymentID,
	})
	if err != nil {
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
