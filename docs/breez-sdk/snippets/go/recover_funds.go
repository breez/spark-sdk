package example

import (
	"encoding/hex"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func FetchRecoverableFunds(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: recoverable-funds
	ensureSynced := false
	info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
		EnsureSynced: &ensureSynced,
	})
	if err != nil {
		return err
	}

	if info.RecoverableFundsSats > 0 {
		log.Printf("%d sats can be recovered on-chain", info.RecoverableFundsSats)
	}
	// ANCHOR_END: recoverable-funds

	return nil
}

func PrepareRecovery(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareRecoverFundsResponse, error) {
	// ANCHOR: prepare-recover-funds
	var fundingKind breez_sdk_spark.CpfpFundingKind = breez_sdk_spark.CpfpFundingKindP2wpkh{}
	quote, err := sdk.PrepareRecoverFunds(breez_sdk_spark.PrepareRecoverFundsRequest{
		FeeRateSatPerVbyte: 2,
		FundingKind:        &fundingKind,
		Destination:        "bc1q...your-destination-address",
		Selection:          breez_sdk_spark.ExitLeafSelectionRecoverableOnly{},
	})
	if err != nil {
		return nil, err
	}

	if len(quote.Leaves) == 0 {
		log.Printf("Nothing to recover")
		return &quote, nil
	}
	for _, leaf := range quote.Leaves {
		log.Printf("%s: %d sats, %v", leaf.LeafId, leaf.ValueSats, leaf.Method)
	}
	log.Printf(
		"Recovering %d sats for %d sats in fees",
		quote.RecoverableValueSats, quote.TotalFeeSats,
	)
	if quote.Funding != nil {
		log.Printf("Fund one UTXO of at least %d sats", quote.Funding.SingleUtxoSats)
	}
	// ANCHOR_END: prepare-recover-funds

	return &quote, nil
}

func RecoverCooperatively(sdk *breez_sdk_spark.BreezSdk, quote breez_sdk_spark.PrepareRecoverFundsResponse) error {
	// ANCHOR: recover-cooperatively
	// A quote with funding holds a unilateral exit: prepare the
	// cooperative leaves alone to recover them without it.
	if quote.Funding != nil {
		var leafIds []string
		for _, leaf := range quote.Leaves {
			if leaf.Method == breez_sdk_spark.RecoveryMethodCooperative {
				leafIds = append(leafIds, leaf.LeafId)
			}
		}
		if len(leafIds) == 0 {
			return nil
		}
		cooperativeQuote, err := sdk.PrepareRecoverFunds(breez_sdk_spark.PrepareRecoverFundsRequest{
			FeeRateSatPerVbyte: quote.FeeRateSatPerVbyte,
			FundingKind:        nil,
			Destination:        quote.Destination,
			Selection:          breez_sdk_spark.ExitLeafSelectionSpecific{LeafIds: leafIds},
		})
		if err != nil {
			return err
		}
		quote = cooperativeQuote
	}
	response, err := sdk.RecoverFunds(breez_sdk_spark.RecoverFundsRequest{
		Prepared:      quote,
		FundingInputs: []breez_sdk_spark.CpfpInput{},
	}, nil)
	if err != nil {
		return err
	}

	// Keep the whole response: CheckRecoverFunds follows the recovery from it.
	for _, tx := range response.Transactions {
		log.Printf("Broadcast %s: %s", tx.Txid, tx.TxHex)
	}
	for _, failure := range response.Failed {
		log.Printf("Leaf %s was not recovered: %v", failure.LeafId, failure.Error)
	}
	// ANCHOR_END: recover-cooperatively

	return nil
}

func RecoverWithFunding(sdk *breez_sdk_spark.BreezSdk, quote breez_sdk_spark.PrepareRecoverFundsResponse) (*breez_sdk_spark.RecoverFundsResponse, error) {
	// ANCHOR: recover-funds
	secretKeyBytes, err := hex.DecodeString("your-secret-key-hex")
	if err != nil {
		return nil, err
	}
	signer, err := breez_sdk_spark.SingleKeyCpfpSigner(secretKeyBytes)
	if err != nil {
		return nil, err
	}

	response, err := sdk.RecoverFunds(breez_sdk_spark.RecoverFundsRequest{
		Prepared: quote,
		FundingInputs: []breez_sdk_spark.CpfpInput{
			breez_sdk_spark.CpfpInputP2wpkh{
				Txid:      "your-utxo-txid",
				Vout:      0,
				ValueSats: 50_000,
				Pubkey:    "your-compressed-pubkey-hex",
			},
		},
	}, &signer)
	if err != nil {
		return nil, err
	}

	// Keep the whole response: CheckRecoverFunds follows the recovery from it.
	for _, tx := range response.Transactions {
		if tx.CsvTimelockBlocks != nil {
			log.Printf(
				"%s: wait %d blocks after its parents confirm",
				tx.Txid, *tx.CsvTimelockBlocks,
			)
		}
	}
	// ANCHOR_END: recover-funds

	return &response, nil
}

func CheckRecovery(sdk *breez_sdk_spark.BreezSdk, stored breez_sdk_spark.RecoverFundsResponse) error {
	// ANCHOR: check-recover-funds
	checked, err := sdk.CheckRecoverFunds(breez_sdk_spark.CheckRecoverFundsRequest{
		Recovery: stored,
	})
	if err != nil {
		return err
	}

	// Store this one in place of the one you had.
	recovery := checked.Recovery

	switch verdict := checked.Verdict.(type) {
	case breez_sdk_spark.RecoveryVerdictValid:
		for _, tx := range recovery.Transactions {
			if _, ready := tx.Status.(breez_sdk_spark.ExitTransactionStatusReady); ready {
				log.Printf("ready to broadcast: %s", tx.Txid)
			}
		}
	case breez_sdk_spark.RecoveryVerdictDone:
		log.Printf("Every transaction confirmed: the recovery is done")
	case breez_sdk_spark.RecoveryVerdictRedo:
		// Prepare and build again, naming the same leaves. Pass
		// recovery.FundingInputs back and the SDK follows them to whatever
		// they have become.
		log.Printf("Build the recovery again: %v", verdict.Reason)
	}
	// ANCHOR_END: check-recover-funds

	return nil
}

func BackUpExitState(sdk *breez_sdk_spark.BreezSdk) (string, error) {
	// ANCHOR: export-exit-state
	exported, err := sdk.ExportUnilateralExitState()
	if err != nil {
		return "", err
	}

	// Keep the state somewhere the wallet's own storage cannot take with it.
	log.Printf("Exit state is %d bytes", len(exported.ExitState))
	// ANCHOR_END: export-exit-state

	return exported.ExitState, nil
}

func RestoreExitState(sdk *breez_sdk_spark.BreezSdk, exitState string) error {
	// ANCHOR: import-exit-state
	imported, err := sdk.ImportUnilateralExitState(breez_sdk_spark.ImportUnilateralExitStateRequest{
		ExitState: exitState,
	})
	if err != nil {
		return err
	}

	log.Printf(
		"Imported %d leaves, skipped %d",
		imported.ImportedLeaves, imported.SkippedForeignLeaves,
	)
	// ANCHOR_END: import-exit-state

	return nil
}

func CollectExitData(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: sync-exit-data
	// With automatic collection off, an explicit sync is what collects the data
	// a unilateral exit needs, and it waits for the collection to finish. Needs
	// the Spark operators reachable, so run it on a schedule rather than at the
	// moment an exit is needed.
	_, err := sdk.SyncWallet(breez_sdk_spark.SyncWalletRequest{})
	if err != nil {
		return err
	}
	// ANCHOR_END: sync-exit-data

	return nil
}

// ANCHOR: custom-cpfp-signer
type MyFundingSigner struct{}

func (MyFundingSigner) SignPsbt(psbtBytes []byte) ([]byte, error) {
	signedPsbtBytes, err := signWithFundingKeys(psbtBytes)
	if err != nil {
		return nil, err
	}
	return signedPsbtBytes, nil
}

func signWithFundingKeys(psbtBytes []byte) ([]byte, error) {
	return psbtBytes, nil
}

// ANCHOR_END: custom-cpfp-signer
