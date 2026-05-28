package com.example.kotlinmpplib

import breez_sdk_spark.*

class Optimize {
    suspend fun runFullOptimization(sdk: BreezSdk) {
        // ANCHOR: optimize-leaves-full
        val outcome = sdk.optimizeLeaves(OptimizeLeavesRequest(mode = OptimizationMode.FULL)).outcome

        when (outcome) {
            is OptimizationOutcome.Completed -> {
                if (outcome.roundsExecuted == 0u) {
                    // Log.v("Breez", "Optimization skipped — wallet already optimal")
                } else {
                    // Log.v("Breez", "Optimization completed in ${outcome.roundsExecuted} rounds")
                }
            }
            is OptimizationOutcome.InProgress -> {
                // Full mode runs to completion in one call, so InProgress is
                // not reachable here.
                throw IllegalStateException("Full mode never returns InProgress")
            }
        }
        // ANCHOR_END: optimize-leaves-full
    }

    suspend fun runOptimizationOneRoundAtATime(sdk: BreezSdk) {
        // ANCHOR: optimize-leaves-single-round
        var roundsExecuted: UInt = 0u
        while (true) {
            val outcome = sdk.optimizeLeaves(OptimizeLeavesRequest(mode = OptimizationMode.SINGLE_ROUND)).outcome
            when (outcome) {
                is OptimizationOutcome.InProgress -> {
                    roundsExecuted += 1u
                    // Log.v("Breez", "Executed round $roundsExecuted")
                }
                is OptimizationOutcome.Completed -> {
                    roundsExecuted += outcome.roundsExecuted
                    if (roundsExecuted == 0u) {
                        // Log.v("Breez", "Optimization skipped — wallet already optimal")
                    } else {
                        // Log.v("Breez", "Optimization done after $roundsExecuted rounds")
                    }
                    break
                }
            }
        }
        // ANCHOR_END: optimize-leaves-single-round
    }

    fun handleAutoOptimizationEvent(optimizationEvent: AutoOptimizationEvent) {
        // ANCHOR: auto-optimization-events
        when (optimizationEvent) {
            is AutoOptimizationEvent.Started -> {
                // Log.v("Breez", "Auto-optimization started with ${optimizationEvent.totalRounds} rounds")
            }
            is AutoOptimizationEvent.RoundCompleted -> {
                // Log.v("Breez", "Auto-optimization round ${optimizationEvent.currentRound} of ${optimizationEvent.totalRounds} completed")
            }
            is AutoOptimizationEvent.Completed -> {
                // Log.v("Breez", "Auto-optimization completed successfully")
            }
            is AutoOptimizationEvent.Cancelled -> {
                // Log.v("Breez", "Auto-optimization was cancelled")
            }
            is AutoOptimizationEvent.Failed -> {
                // Log.v("Breez", "Auto-optimization failed: ${optimizationEvent.error}")
            }
            is AutoOptimizationEvent.Skipped -> {
                // Log.v("Breez", "Auto-optimization was skipped because leaves are already optimal")
            }
        }
        // ANCHOR_END: auto-optimization-events
    }
}
