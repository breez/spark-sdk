import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> startOptimization(BreezClient client) async {
  // ANCHOR: start-optimization
  client.startLeafOptimization();
  // ANCHOR_END: start-optimization
}

Future<void> cancelOptimization(BreezClient client) async {
  // ANCHOR: cancel-optimization
  await client.cancelLeafOptimization();
  // ANCHOR_END: cancel-optimization
}

Future<void> getOptimizationProgress(BreezClient client) async {
  // ANCHOR: get-optimization-progress
  var progress = client.getLeafOptimizationProgress();

  print("Optimization is running: ${progress.isRunning}");
  print("Current round: ${progress.currentRound}");
  print("Total rounds: ${progress.totalRounds}");
  // ANCHOR_END: get-optimization-progress
}

void optimizationEvents(LeafOptimizationEvent optimizationEvent) {
  // ANCHOR: optimization-events
  switch (optimizationEvent) {
    case LeafOptimizationEvent_Started(totalRounds: var totalRounds):
      print("Optimization started with $totalRounds rounds");
      break;
    case LeafOptimizationEvent_RoundCompleted(currentRound: var currentRound, totalRounds: var totalRounds):
      print("Optimization round $currentRound of $totalRounds completed");
      break;
    case LeafOptimizationEvent_Completed():
      print("Optimization completed successfully");
      break;
    case LeafOptimizationEvent_Cancelled():
      print("Optimization was cancelled");
      break;
    case LeafOptimizationEvent_Failed(error: var error):
      print("Optimization failed: $error");
      break;
    case LeafOptimizationEvent_Skipped():
      print("Optimization was skipped because leaves are already optimal");
      break;
  }
}