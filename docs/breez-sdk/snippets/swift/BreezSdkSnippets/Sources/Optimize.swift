import BreezSdkSpark

func runFullOptimization(sdk: BreezSdk) async throws {
    // ANCHOR: optimize-leaves-full
    let outcome = try await sdk.optimizeLeaves(request: OptimizeLeavesRequest(mode: .full)).outcome

    switch outcome {
    case .completed(let roundsExecuted):
        if roundsExecuted == 0 {
            print("Optimization skipped — wallet already optimal")
        } else {
            print("Optimization completed in \(roundsExecuted) rounds")
        }
    case .inProgress:
        // Full mode runs to completion in one call, so InProgress is
        // not reachable here.
        fatalError("Full mode never returns InProgress")
    }
    // ANCHOR_END: optimize-leaves-full
}

func runOptimizationOneRoundAtATime(sdk: BreezSdk) async throws {
    // ANCHOR: optimize-leaves-single-round
    var roundsExecuted: UInt32 = 0
    loop: while true {
        let outcome = try await sdk.optimizeLeaves(
            request: OptimizeLeavesRequest(mode: .singleRound)
        ).outcome

        switch outcome {
        case .inProgress:
            roundsExecuted += 1
            print("Executed round \(roundsExecuted)")
        case .completed(let thisRound):
            roundsExecuted += thisRound
            if roundsExecuted == 0 {
                print("Optimization skipped — wallet already optimal")
            } else {
                print("Optimization done after \(roundsExecuted) rounds")
            }
            break loop
        }
    }
    // ANCHOR_END: optimize-leaves-single-round
}

func handleAutoOptimizationEvent(event: AutoOptimizationEvent) {
    // ANCHOR: auto-optimization-events
    switch event {
    case .started(let totalRounds):
        print("Auto-optimization started with \(totalRounds) rounds")
    case .roundCompleted(let currentRound, let totalRounds):
        print("Auto-optimization round \(currentRound) of \(totalRounds) completed")
    case .completed:
        print("Auto-optimization completed successfully")
    case .cancelled:
        print("Auto-optimization was cancelled")
    case .failed(let error):
        print("Auto-optimization failed: \(error)")
    case .skipped:
        print("Auto-optimization was skipped because leaves are already optimal")
    }
    // ANCHOR_END: auto-optimization-events
}
