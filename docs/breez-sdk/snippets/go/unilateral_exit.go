package example

import (
	"encoding/hex"
	"fmt"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareExit(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareUnilateralExitResponse, error) {
	// ANCHOR: prepare-unilateral-exit
	// Create a signer from your UTXO private key (32-byte secret key)
	secretKeyBytes, err := hex.DecodeString("your-secret-key-hex")
	if err != nil {
		return nil, err
	}
	signer, err := breez_sdk_spark.NewSingleKeySigner(secretKeyBytes)
	if err != nil {
		return nil, err
	}

	response, err := sdk.PrepareUnilateralExit(breez_sdk_spark.PrepareUnilateralExitRequest{
		FeeRate: 2,
		Inputs: []breez_sdk_spark.UnilateralExitCpfpInput{
			breez_sdk_spark.UnilateralExitCpfpInputP2wpkh{
				Txid:   "your-utxo-txid",
				Vout:   0,
				Value:  50_000,
				Pubkey: "your-compressed-pubkey-hex",
			},
		},
		Destination: "bc1q...your-destination-address",
	}, signer)
	if err != nil {
		return nil, err
	}

	// The SDK automatically selects which leaves are profitable to exit.
	for _, leaf := range response.SelectedLeaves {
		log.Printf("Leaf %s: %d sats (exit cost: ~%d sats)", leaf.Id, leaf.Value, leaf.EstimatedCost)
	}

	// The response contains signed transactions ready to broadcast:
	// - response.Transactions: parent/child transaction pairs per leaf
	// - response.SweepTxHex: signed sweep transaction for the final step
	// Change from CPFP fee-bumping always goes back to the first input's address.
	for _, leaf := range response.Transactions {
		for _, pair := range leaf.TxCpfpPairs {
			if pair.CsvTimelockBlocks != nil {
				fmt.Printf("Timelock: wait %d blocks\n", *pair.CsvTimelockBlocks)
			}
			// pair.ParentTxHex: pre-signed Spark transaction
			// pair.ChildTxHex: signed CPFP transaction — broadcast alongside parent
		}
	}
	// ANCHOR_END: prepare-unilateral-exit
	return &response, nil
}
