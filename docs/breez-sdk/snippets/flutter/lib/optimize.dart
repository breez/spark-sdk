import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> startOptimization(BreezSdk sdk) async {
  // ANCHOR: start-optimization
  sdk.startLeafOptimization();
  // ANCHOR_END: start-optimization
}

Future<void> cancelOptimization(BreezSdk sdk) async {
  // ANCHOR: cancel-optimization
  await sdk.cancelLeafOptimization();
  // ANCHOR_END: cancel-optimization
}

Future<void> getOptimizationProgress(BreezSdk sdk) async {
  // ANCHOR: get-optimization-progress
  var progress = sdk.getLeafOptimizationProgress();

  print("Optimization is running: ${progress.isRunning}");
  print("Current round: ${progress.currentRound}");
  print("Total rounds: ${progress.totalRounds}");
  // ANCHOR_END: get-optimization-progress
}

void optimizationEvents(OptimizationEvent optimizationEvent) {
  // ANCHOR: optimization-events
  switch (optimizationEvent) {
    case OptimizationEvent_Started(totalRounds: var totalRounds):
      print("Optimization started with $totalRounds rounds");
      break;
    case OptimizationEvent_RoundCompleted(currentRound: var currentRound, totalRounds: var totalRounds):
      print("Optimization round $currentRound of $totalRounds completed");
      break;
    case OptimizationEvent_Completed():
      print("Optimization completed successfully");
      break;
    case OptimizationEvent_Cancelled():
      print("Optimization was cancelled");
      break;
    case OptimizationEvent_Failed(error: var error):
      print("Optimization failed: $error");
      break;
    case OptimizationEvent_Skipped():
      print("Optimization was skipped because leaves are already optimal");
      break;
  }
}