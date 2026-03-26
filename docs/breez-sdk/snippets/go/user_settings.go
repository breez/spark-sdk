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
	sparkPrivateModeEnabled := true
	err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
		SparkPrivateModeEnabled: &sparkPrivateModeEnabled,
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
