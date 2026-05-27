package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func RunFullOptimization(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: optimize-leaves-full
	outcome, err := sdk.OptimizeLeaves(nil)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
		}
		return err
	}

	switch o := outcome.(type) {
	case breez_sdk_spark.OptimizationOutcomeCompleted:
		if o.RoundsExecuted == 0 {
			log.Printf("Optimization skipped — wallet already optimal")
		} else {
			log.Printf("Optimization completed in %v rounds", o.RoundsExecuted)
		}
	case breez_sdk_spark.OptimizationOutcomeInProgress:
		// Full mode runs to completion in one call, so InProgress is
		// not reachable here.
		log.Panicf("Full mode never returns InProgress")
	}
	// ANCHOR_END: optimize-leaves-full
	return nil
}

func RunOptimizationOneRoundAtATime(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: optimize-leaves-single-round
	var roundsExecuted uint32 = 0
	for {
		outcome, err := sdk.OptimizeLeaves(&breez_sdk_spark.OptimizeLeavesOptions{
			Mode: breez_sdk_spark.OptimizationModeSingleRound,
		})
		if err != nil {
			var sdkErr *breez_sdk_spark.SdkError
			if errors.As(err, &sdkErr) {
				// Handle SdkError - can inspect specific variants if needed
			}
			return err
		}

		switch o := outcome.(type) {
		case breez_sdk_spark.OptimizationOutcomeInProgress:
			roundsExecuted += 1
			log.Printf("Executed round %v", roundsExecuted)
		case breez_sdk_spark.OptimizationOutcomeCompleted:
			roundsExecuted += o.RoundsExecuted
			if roundsExecuted == 0 {
				log.Printf("Optimization skipped — wallet already optimal")
			} else {
				log.Printf("Optimization done after %v rounds", roundsExecuted)
			}
			return nil
		}
	}
	// ANCHOR_END: optimize-leaves-single-round
}

func HandleAutoOptimizationEvent(optimizationEvent breez_sdk_spark.AutoOptimizationEvent) {
	// ANCHOR: auto-optimization-events
	switch event := optimizationEvent.(type) {
	case breez_sdk_spark.AutoOptimizationEventStarted:
		log.Printf("Auto-optimization started with %v rounds", event.TotalRounds)
	case breez_sdk_spark.AutoOptimizationEventRoundCompleted:
		log.Printf("Auto-optimization round %v of %v completed", event.CurrentRound, event.TotalRounds)
	case breez_sdk_spark.AutoOptimizationEventCompleted:
		log.Printf("Auto-optimization completed successfully")
	case breez_sdk_spark.AutoOptimizationEventCancelled:
		log.Printf("Auto-optimization was cancelled")
	case breez_sdk_spark.AutoOptimizationEventFailed:
		log.Printf("Auto-optimization failed: %v", event.Error)
	case breez_sdk_spark.AutoOptimizationEventSkipped:
		log.Printf("Auto-optimization was skipped because leaves are already optimal")
	}
	// ANCHOR_END: auto-optimization-events
}
