package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func StartOptimization(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: start-optimization
	sdk.StartLeafOptimization()
	// ANCHOR_END: start-optimization
	return nil
}

func CancelOptimization(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: cancel-optimization
	err := sdk.CancelLeafOptimization()
	if err != nil {
		return err
	}
	// ANCHOR_END: cancel-optimization
	return nil
}

func GetOptimizationProgress(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: get-optimization-progress
	progress := sdk.GetLeafOptimizationProgress()

	log.Printf("Optimization is running: %v\n", progress.IsRunning)
	log.Printf("Current round: %v\n", progress.CurrentRound)
	log.Printf("Total rounds: %v\n", progress.TotalRounds)
	// ANCHOR_END: get-optimization-progress
	return nil
}

func OptimizationEvents(optimizationEvent breez_sdk_spark.OptimizationEvent) {
	// ANCHOR: optimization-events
	switch event := optimizationEvent.(type) {
	case breez_sdk_spark.OptimizationEventStarted:
		log.Printf("Optimization started with %v rounds\n", event.TotalRounds)
	case breez_sdk_spark.OptimizationEventRoundCompleted:
		log.Printf("Optimization round %v of %v completed\n", event.CurrentRound, event.TotalRounds)
	case breez_sdk_spark.OptimizationEventCompleted:
		log.Printf("Optimization completed successfully\n")
	case breez_sdk_spark.OptimizationEventCancelled:
		log.Printf("Optimization was cancelled\n")
	case breez_sdk_spark.OptimizationEventFailed:
		log.Printf("Optimization failed: %v\n", event.Error)
	case breez_sdk_spark.OptimizationEventSkipped:
		log.Printf("Optimization was skipped because leaves are already optimal\n")
	}
	// ANCHOR_END: optimization-events
}
