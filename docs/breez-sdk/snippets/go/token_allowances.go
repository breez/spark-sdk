package example

import (
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func CreateTokenAllowance(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: create-token-allowance
	maxPerPayment := breez_sdk_spark.TokenAllowanceLimitAmount{
		Amount: new(big.Int).SetInt64(5_000_000),
	}
	maxTotal := breez_sdk_spark.TokenAllowanceLimitAmount{
		Amount: new(big.Int).SetInt64(100_000_000),
	}

	response, err := sdk.CreateTokenAllowance(breez_sdk_spark.CreateTokenAllowanceRequest{
		SpenderPublicKey:  "<spender identity public key>",
		TokenIdentifier:   "<token identifier>",
		MaxPerPayment:     maxPerPayment,
		MaxTotal:          maxTotal,
		ExpiryTime:        1_798_761_600,
		AllowedRecipients: []string{},
	})
	if err != nil {
		return err
	}
	log.Printf("Allowance id: %v", response.Allowance.Id)
	// ANCHOR_END: create-token-allowance
	return nil
}

func ListTokenAllowances(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: list-token-allowances
	includeInactive := false

	response, err := sdk.ListTokenAllowances(breez_sdk_spark.ListTokenAllowancesRequest{
		Role:                  breez_sdk_spark.TokenAllowanceRoleOwner,
		CounterpartyPublicKey: nil,
		TokenIdentifier:       nil,
		IncludeInactive:       &includeInactive,
		Offset:                nil,
		Limit:                 nil,
	})
	if err != nil {
		return err
	}
	for _, allowance := range response.Allowances {
		log.Printf("%v: spent %v", allowance.Id, allowance.SpentAmount)
	}
	// ANCHOR_END: list-token-allowances
	return nil
}

func RevokeTokenAllowance(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: revoke-token-allowance
	err := sdk.RevokeTokenAllowance(breez_sdk_spark.RevokeTokenAllowanceRequest{
		AllowanceId: "<allowance id>",
	})
	if err != nil {
		return err
	}
	// ANCHOR_END: revoke-token-allowance
	return nil
}

func PullPayment(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: pull-payment
	prepareResponse, err := sdk.PreparePullPayment(breez_sdk_spark.PreparePullPaymentRequest{
		PayerPublicKey:  "<payer identity public key>",
		TokenIdentifier: "<token identifier>",
		Receivers: []breez_sdk_spark.PullReceiver{
			{
				Amount:            new(big.Int).SetInt64(5_000_000),
				ReceiverPublicKey: nil,
			},
		},
	})
	if err != nil {
		return err
	}
	log.Printf("Pulling %v", prepareResponse.Amount)

	response, err := sdk.PullPayment(breez_sdk_spark.PullPaymentRequest{
		PrepareResponse: prepareResponse,
	})
	if err != nil {
		return err
	}
	log.Printf("Pull transaction: %v", response.TxHash)
	// ANCHOR_END: pull-payment
	return nil
}
