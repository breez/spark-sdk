package example

import (
	"encoding/hex"
	"fmt"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func QuoteExit(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareUnilateralExitResponse, error) {
	// ANCHOR: prepare-unilateral-exit
	quote, err := sdk.PrepareUnilateralExit(breez_sdk_spark.PrepareUnilateralExitRequest{
		FeeRateSatPerVbyte: 2,
		FundingKind:        breez_sdk_spark.CpfpFundingKindP2wpkh{},
		Destination:        "bc1q...your-destination-address",
		Selection:          breez_sdk_spark.ExitLeafSelectionAuto{},
	})
	if err != nil {
		return nil, err
	}

	log.Printf("Recovering %d sats for %d sats in fees", quote.RecoverableValueSat, quote.TotalFeeSat)
	log.Printf("Fund a single UTXO of at least %d sats", quote.SingleUtxoFundingSat)
	// ANCHOR_END: prepare-unilateral-exit
	return &quote, nil
}

func BuildExit(sdk *breez_sdk_spark.BreezSdk, quote breez_sdk_spark.PrepareUnilateralExitResponse) error {
	// ANCHOR: unilateral-exit
	secretKeyBytes, err := hex.DecodeString("your-secret-key-hex")
	if err != nil {
		return err
	}
	signer, err := breez_sdk_spark.SingleKeyCpfpSigner(secretKeyBytes)
	if err != nil {
		return err
	}

	response, err := sdk.UnilateralExit(breez_sdk_spark.UnilateralExitRequest{
		Prepared: quote,
		FundingInputs: []breez_sdk_spark.CpfpInput{
			breez_sdk_spark.CpfpInputP2wpkh{
				Txid:   "your-utxo-txid",
				Vout:   0,
				Value:  50_000,
				Pubkey: "your-compressed-pubkey-hex",
			},
		},
	}, signer)
	if err != nil {
		return err
	}

	for _, tx := range response.Transactions {
		if tx.CsvTimelockBlocks != nil {
			fmt.Printf("%s: wait %d blocks after its parents confirm\n", tx.Txid, *tx.CsvTimelockBlocks)
		}
	}
	// ANCHOR_END: unilateral-exit
	return nil
}

// ANCHOR: custom-cpfp-signer
type MyCpfpSigner struct{}

func (MyCpfpSigner) SignPsbt(psbtBytes []byte) ([]byte, error) {
	return signPsbtWithYourKeys(psbtBytes)
}

func signPsbtWithYourKeys(psbtBytes []byte) ([]byte, error) {
	return psbtBytes, nil
}

// ANCHOR_END: custom-cpfp-signer
