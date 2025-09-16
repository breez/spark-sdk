package example

import (
	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

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
		Description: description,
	}

	addressInfo, err := sdk.RegisterLightningAddress(request)
	if err != nil {
		return nil, err
	}

	_ = addressInfo.LightningAddress
	_ = addressInfo.Lnurl
	// ANCHOR_END: register-lightning-address

	return addressInfo, nil
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
