using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Optimize
    {
        void StartOptimization(BreezClient client)
        {
            // ANCHOR: start-optimization
            client.Optimization().Start();
            // ANCHOR_END: start-optimization
        }

        async Task CancelOptimization(BreezClient client)
        {
            // ANCHOR: cancel-optimization
            await client.Optimization().Cancel();
            // ANCHOR_END: cancel-optimization
        }

        void GetOptimizationProgress(BreezClient client)
        {
            // ANCHOR: get-optimization-progress
            var progress = client.Optimization().Progress();

            Console.WriteLine($"Optimization is running: {progress.isRunning}");
            Console.WriteLine($"Current round: {progress.currentRound}");
            Console.WriteLine($"Total rounds: {progress.totalRounds}");
            // ANCHOR_END: get-optimization-progress
        }

        void OptimizationEvents(LeafOptimizationEvent optimizationEvent)
        {
            // ANCHOR: optimization-events
            switch (optimizationEvent)
            {
                case LeafOptimizationEvent.Started { totalRounds: var totalRounds }:
                    Console.WriteLine($"Optimization started with {totalRounds} rounds");
                    break;
                case LeafOptimizationEvent.RoundCompleted { currentRound: var currentRound, totalRounds: var totalRounds }:
                    Console.WriteLine($"Optimization round {currentRound} of {totalRounds} completed");
                    break;
                case LeafOptimizationEvent.Completed:
                    Console.WriteLine("Optimization completed successfully");
                    break;
                case LeafOptimizationEvent.Cancelled:
                    Console.WriteLine("Optimization was cancelled");
                    break;
                case LeafOptimizationEvent.Failed { error: var error }:
                    Console.WriteLine($"Optimization failed: {error}");
                    break;
                case LeafOptimizationEvent.Skipped:
                    Console.WriteLine("Optimization was skipped because leaves are already optimal");
                    break;
            }
        }
    }
}