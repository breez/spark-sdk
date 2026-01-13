package com.example.kotlinmpplib

import breez_sdk_spark.*

class Optimize {
    fun startOptimization(sdk: BreezSdk) {
        // ANCHOR: start-optimization
        sdk.startLeafOptimization()
        // ANCHOR_END: start-optimization
    }

    suspend fun cancelOptimization(sdk: BreezSdk) {
        // ANCHOR: cancel-optimization
        try {
            sdk.cancelLeafOptimization()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: cancel-optimization
    }

    fun getOptimizationProgress(sdk: BreezSdk) {
        // ANCHOR: get-optimization-progress
        val progress = sdk.getLeafOptimizationProgress()

        println("Optimization is running: ${progress.isRunning}")
        println("Current round: ${progress.currentRound}")
        println("Total rounds: ${progress.totalRounds}")
        // ANCHOR_END: get-optimization-progress
    }

    fun optimizationEvents(optimizationEvent: OptimizationEvent) {
        // ANCHOR: optimization-events
        when (optimizationEvent) {
            is OptimizationEvent.Started -> {
                // Log.v("Breez", "Optimization started with ${optimizationEvent.totalRounds} rounds")
            }
            is OptimizationEvent.RoundCompleted -> {
                // Log.v("Breez", "Optimization round ${optimizationEvent.currentRound} of ${optimizationEvent.totalRounds} completed")
            }
            is OptimizationEvent.Completed -> {
                // Log.v("Breez", "Optimization completed successfully")
            }
            is OptimizationEvent.Cancelled -> {
                // Log.v("Breez", "Optimization was cancelled")
            }
            is OptimizationEvent.Failed -> {
                // Log.v("Breez", "Optimization failed: ${optimizationEvent.error}")
            }
            is OptimizationEvent.Skipped -> {
                // Log.v("Breez", "Optimization was skipped because leaves are already optimal")
            }
        }
        // ANCHOR_END: optimization-events
    }
}