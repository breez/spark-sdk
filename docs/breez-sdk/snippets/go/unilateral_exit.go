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
		FeeRateSatPerVbyte: 2,
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
	for _, leaf := range response.Leaves {
		log.Printf("Leaf %s: %d sats (exit cost: ~%d sats)", leaf.LeafId, leaf.Value, leaf.EstimatedCost)
		for _, tx := range leaf.Transactions {
			if tx.CsvTimelockBlocks != nil {
				fmt.Printf("Timelock: wait %d blocks\n", *tx.CsvTimelockBlocks)
			}
			// tx.TxHex: pre-signed Spark transaction
			// tx.CpfpTxHex: signed CPFP transaction — broadcast alongside parent
		}
	}

	if len(response.UnverifiedNodeIds) > 0 {
		log.Printf("Warning: could not verify confirmation status for %d nodes", len(response.UnverifiedNodeIds))
	}
	// ANCHOR_END: prepare-unilateral-exit
	return &response, nil
}
