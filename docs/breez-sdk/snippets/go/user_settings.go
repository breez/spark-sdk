package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func GetUserSettings(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: get-user-settings
	userSettings, err := sdk.GetUserSettings()

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
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

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}
	// ANCHOR_END: update-user-settings
	return nil
}
