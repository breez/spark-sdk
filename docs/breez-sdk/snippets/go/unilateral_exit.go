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

func CheckExit(sdk *breez_sdk_spark.BreezSdk, stored breez_sdk_spark.UnilateralExitResponse) error {
	// ANCHOR: check-unilateral-exit
	checked, err := sdk.CheckUnilateralExit(breez_sdk_spark.CheckUnilateralExitRequest{
		Exit: stored,
	})
	if err != nil {
		return err
	}

	// Store this one in place of the one you had.
	exit := checked.Exit

	switch verdict := checked.Verdict.(type) {
	case breez_sdk_spark.UnilateralExitVerdictValid:
		for _, tx := range exit.Transactions {
			if _, ready := tx.Status.(breez_sdk_spark.ExitTransactionStatusReady); ready {
				log.Printf("ready to broadcast: %s", tx.Txid)
			}
		}
	case breez_sdk_spark.UnilateralExitVerdictDone:
		log.Printf("The exit finished: %d sats recovered", exit.RecoverableValueSat)
	case breez_sdk_spark.UnilateralExitVerdictRedo:
		// Quote and build again, naming the same leaves. Pass exit.FundingInputs
		// back and the SDK follows them to whatever they have become.
		log.Printf("Build the exit again: %v", verdict.Reason)
	}
	// ANCHOR_END: check-unilateral-exit

	return nil
}

func ExportExitState(sdk *breez_sdk_spark.BreezSdk) (string, error) {
	// ANCHOR: export-unilateral-exit-state
	exported, err := sdk.ExportUnilateralExitState()
	if err != nil {
		return "", err
	}

	// Keep the state somewhere the wallet's own storage cannot take with it.
	log.Printf("Exit state is %v bytes", len(exported.ExitState))
	// ANCHOR_END: export-unilateral-exit-state

	return exported.ExitState, nil
}

func ImportExitState(sdk *breez_sdk_spark.BreezSdk, exitState string) error {
	// ANCHOR: import-unilateral-exit-state
	imported, err := sdk.ImportUnilateralExitState(breez_sdk_spark.ImportUnilateralExitStateRequest{
		ExitState: exitState,
	})
	if err != nil {
		return err
	}

	log.Printf("Imported %d leaves, skipped %d", imported.ImportedLeaves, imported.SkippedForeignLeaves)
	// ANCHOR_END: import-unilateral-exit-state

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
