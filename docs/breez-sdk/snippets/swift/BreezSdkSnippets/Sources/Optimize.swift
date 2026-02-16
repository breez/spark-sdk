import BreezSdkSpark

func startOptimization(client: BreezClient) {
    // ANCHOR: start-optimization
    client.startLeafOptimization()
    // ANCHOR_END: start-optimization
}

func cancelOptimization(client: BreezClient) async throws {
    // ANCHOR: cancel-optimization
    do {
        try await client.cancelLeafOptimization()
    } catch {
        print("Failed to cancel optimization: \(error)")
    }
    // ANCHOR_END: cancel-optimization
}

func getOptimizationProgress(client: BreezClient) {
    // ANCHOR: get-optimization-progress
    let progress = client.getLeafOptimizationProgress()

    print("Optimization is running: \(progress.isRunning)")
    print("Current round: \(progress.currentRound)")
    print("Total rounds: \(progress.totalRounds)")
    // ANCHOR_END: get-optimization-progress
}

func optimizationEvents(event: LeafOptimizationEvent) {
    // ANCHOR: optimization-events
    switch event {
        case .started(let totalRounds):
            print("Optimization started with \(totalRounds) rounds")
        case .roundCompleted(let currentRound, let totalRounds):
            print("Optimization round \(currentRound) of \(totalRounds) completed")
        case .completed:
            print("Optimization completed successfully")
        case .cancelled:
            print("Optimization was cancelled")
        case .failed(let error):
            print("Optimization failed: \(error)")
        case .skipped:
            print("Optimization was skipped because leaves are already optimal")
    }
    // ANCHOR_END: optimization-events
}