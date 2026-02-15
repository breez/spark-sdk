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

    fun optimizationEvents(optimizationEvent: LeafOptimizationEvent) {
        // ANCHOR: optimization-events
        when (optimizationEvent) {
            is LeafOptimizationEvent.Started -> {
                // Log.v("Breez", "Optimization started with ${optimizationEvent.totalRounds} rounds")
            }
            is LeafOptimizationEvent.RoundCompleted -> {
                // Log.v("Breez", "Optimization round ${optimizationEvent.currentRound} of ${optimizationEvent.totalRounds} completed")
            }
            is LeafOptimizationEvent.Completed -> {
                // Log.v("Breez", "Optimization completed successfully")
            }
            is LeafOptimizationEvent.Cancelled -> {
                // Log.v("Breez", "Optimization was cancelled")
            }
            is LeafOptimizationEvent.Failed -> {
                // Log.v("Breez", "Optimization failed: ${optimizationEvent.error}")
            }
            is LeafOptimizationEvent.Skipped -> {
                // Log.v("Breez", "Optimization was skipped because leaves are already optimal")
            }
        }
        // ANCHOR_END: optimization-events
    }
}