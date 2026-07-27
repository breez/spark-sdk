package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetUserSettings(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: get-user-settings
	userSettings, err := sdk.GetUserSettings()

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}

	log.Printf("User settings: %v", userSettings)
	// ANCHOR_END: get-user-settings
	return nil
}

func UpdateUserSettings(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: update-user-settings
	// Fields left nil are not changed. Settings that take an enum value
	// accept Set to assign one, or Unset to clear it.
	sparkPrivateModeEnabled := true
	masterIdentityPublicKey := breez_sdk_spark.SparkMasterIdentityPublicKey(
		breez_sdk_spark.SparkMasterIdentityPublicKeySet{
			PublicKey: "<hex encoded public key>",
		},
	)
	err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
		SparkPrivateModeEnabled:      &sparkPrivateModeEnabled,
		StableBalanceActiveLabel:     nil,
		SparkMasterIdentityPublicKey: &masterIdentityPublicKey,
	})

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}
	// ANCHOR_END: update-user-settings
	return nil
}

func ActivateStableBalance(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: activate-stable-balance
	activeLabel := breez_sdk_spark.StableBalanceActiveLabel(
		breez_sdk_spark.StableBalanceActiveLabelSet{Label: "USDB"},
	)
	err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
		StableBalanceActiveLabel: &activeLabel,
	})

	if err != nil {
		return err
	}
	// ANCHOR_END: activate-stable-balance
	return nil
}

func DeactivateStableBalance(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: deactivate-stable-balance
	activeLabel := breez_sdk_spark.StableBalanceActiveLabel(
		breez_sdk_spark.StableBalanceActiveLabelUnset{},
	)
	err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
		StableBalanceActiveLabel: &activeLabel,
	})

	if err != nil {
		return err
	}
	// ANCHOR_END: deactivate-stable-balance
	return nil
}
