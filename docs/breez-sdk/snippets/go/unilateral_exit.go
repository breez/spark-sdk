package example

import (
	"fmt"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ListLeavesForExit(sdk *breez_sdk_spark.BreezSdk) ([]breez_sdk_spark.Leaf, error) {
	// ANCHOR: list-leaves
	minValueSats := uint64(10_000)
	response, err := sdk.ListLeaves(breez_sdk_spark.ListLeavesRequest{
		MinValueSats: &minValueSats,
	})
	if err != nil {
		return nil, err
	}

	for _, leaf := range response.Leaves {
		log.Printf("Leaf %s: %d sats", leaf.Id, leaf.Value)
	}
	// ANCHOR_END: list-leaves
	return response.Leaves, nil
}

func PrepareExit(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareUnilateralExitResponse, error) {
	// ANCHOR: prepare-unilateral-exit
	leafIds := []string{"leaf-id-1", "leaf-id-2"}

	response, err := sdk.PrepareUnilateralExit(breez_sdk_spark.PrepareUnilateralExitRequest{
		FeeRate: 2,
		LeafIds: leafIds,
		Utxos: []breez_sdk_spark.UnilateralExitCpfpUtxo{
			{
				Txid:     "your-utxo-txid",
				Vout:     0,
				Value:    50_000,
				Pubkey:   "your-compressed-pubkey-hex",
				UtxoType: breez_sdk_spark.UnilateralExitCpfpUtxoTypeP2wpkh,
			},
		},
		Destination: "bc1q...your-destination-address",
	})
	if err != nil {
		return nil, err
	}

	// The response contains:
	// - response.Leaves: transaction/PSBT pairs to sign and broadcast
	// - response.SweepTxHex: signed sweep transaction for the final step
	for _, leaf := range response.Leaves {
		for _, pair := range leaf.TxCpfpPsbts {
			if pair.CsvTimelockBlocks != nil {
				fmt.Printf("Timelock: wait %d blocks\n", *pair.CsvTimelockBlocks)
			}
			// pair.ParentTxHex: pre-signed Spark transaction
			// pair.ChildPsbtHex: unsigned CPFP PSBT — sign with your UTXO key
		}
	}
	// ANCHOR_END: prepare-unilateral-exit
	return &response, nil
}
