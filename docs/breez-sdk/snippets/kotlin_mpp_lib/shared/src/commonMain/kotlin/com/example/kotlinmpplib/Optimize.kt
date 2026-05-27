package com.example.kotlinmpplib

import breez_sdk_spark.*

class Optimize {
    suspend fun runFullOptimization(sdk: BreezSdk) {
        // ANCHOR: optimize-leaves-full
        val outcome = sdk.optimizeLeaves(null)

        when (outcome) {
            is OptimizationOutcome.Completed -> {
                // Log.v("Breez", "Optimization completed in ${outcome.roundsExecuted} rounds")
            }
            is OptimizationOutcome.Skipped -> {
                // Log.v("Breez", "Optimization skipped — wallet already optimal")
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
            val outcome = sdk.optimizeLeaves(OptimizeLeavesOptions(mode = OptimizationMode.SINGLE_ROUND))
            when (outcome) {
                is OptimizationOutcome.InProgress -> {
                    roundsExecuted += 1u
                    // Log.v("Breez", "Executed round $roundsExecuted")
                }
                is OptimizationOutcome.Completed -> {
                    roundsExecuted += outcome.roundsExecuted
                    // Log.v("Breez", "Optimization done after $roundsExecuted rounds")
                    break
                }
                is OptimizationOutcome.Skipped -> {
                    // Log.v("Breez", "Optimization skipped — wallet already optimal")
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
