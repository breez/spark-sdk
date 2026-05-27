using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Optimize
    {
        async Task RunFullOptimization(BreezSdk sdk)
        {
            // ANCHOR: optimize-leaves-full
            var outcome = await sdk.OptimizeLeaves(null);

            switch (outcome)
            {
                case OptimizationOutcome.Completed { roundsExecuted: var roundsExecuted }:
                    Console.WriteLine($"Optimization completed in {roundsExecuted} rounds");
                    break;
                case OptimizationOutcome.Skipped:
                    Console.WriteLine("Optimization skipped — wallet already optimal");
                    break;
                case OptimizationOutcome.InProgress:
                    // Full mode runs to completion in one call, so InProgress is
                    // not reachable here.
                    throw new InvalidOperationException("Full mode never returns InProgress");
            }
            // ANCHOR_END: optimize-leaves-full
        }

        async Task RunOptimizationOneRoundAtATime(BreezSdk sdk)
        {
            // ANCHOR: optimize-leaves-single-round
            uint roundsExecuted = 0;
            while (true)
            {
                var outcome = await sdk.OptimizeLeaves(new OptimizeLeavesOptions(OptimizationMode.SingleRound));

                if (outcome is OptimizationOutcome.InProgress)
                {
                    roundsExecuted += 1;
                    Console.WriteLine($"Executed round {roundsExecuted}");
                }
                else if (outcome is OptimizationOutcome.Completed { roundsExecuted: var n })
                {
                    roundsExecuted += n;
                    Console.WriteLine($"Optimization done after {roundsExecuted} rounds");
                    break;
                }
                else if (outcome is OptimizationOutcome.Skipped)
                {
                    Console.WriteLine("Optimization skipped — wallet already optimal");
                    break;
                }
            }
            // ANCHOR_END: optimize-leaves-single-round
        }

        void HandleAutoOptimizationEvent(AutoOptimizationEvent optimizationEvent)
        {
            // ANCHOR: auto-optimization-events
            switch (optimizationEvent)
            {
                case AutoOptimizationEvent.Started { totalRounds: var totalRounds }:
                    Console.WriteLine($"Auto-optimization started with {totalRounds} rounds");
                    break;
                case AutoOptimizationEvent.RoundCompleted { currentRound: var currentRound, totalRounds: var totalRounds }:
                    Console.WriteLine($"Auto-optimization round {currentRound} of {totalRounds} completed");
                    break;
                case AutoOptimizationEvent.Completed:
                    Console.WriteLine("Auto-optimization completed successfully");
                    break;
                case AutoOptimizationEvent.Cancelled:
                    Console.WriteLine("Auto-optimization was cancelled");
                    break;
                case AutoOptimizationEvent.Failed { error: var error }:
                    Console.WriteLine($"Auto-optimization failed: {error}");
                    break;
                case AutoOptimizationEvent.Skipped:
                    Console.WriteLine("Auto-optimization was skipped because leaves are already optimal");
                    break;
            }
            // ANCHOR_END: auto-optimization-events
        }
    }
}
