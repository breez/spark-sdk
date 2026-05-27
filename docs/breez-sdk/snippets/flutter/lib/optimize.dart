import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> runFullOptimization(BreezSdk sdk) async {
  // ANCHOR: optimize-leaves-full
  final outcome = await sdk.optimizeLeaves();

  switch (outcome) {
    case OptimizationOutcome_Completed(:final roundsExecuted):
      print("Optimization completed in $roundsExecuted rounds");
      break;
    case OptimizationOutcome_Skipped():
      print("Optimization skipped — wallet already optimal");
      break;
    case OptimizationOutcome_InProgress():
      // Full mode runs to completion in one call, so InProgress is
      // not reachable here.
      throw StateError("Full mode never returns InProgress");
  }
  // ANCHOR_END: optimize-leaves-full
}

Future<void> runOptimizationOneRoundAtATime(BreezSdk sdk) async {
  // ANCHOR: optimize-leaves-single-round
  var roundsExecuted = 0;
  while (true) {
    final outcome = await sdk.optimizeLeaves(
        options: OptimizeLeavesOptions(mode: OptimizationMode.singleRound));
    switch (outcome) {
      case OptimizationOutcome_InProgress():
        roundsExecuted += 1;
        print("Executed round $roundsExecuted");
        break;
      case OptimizationOutcome_Completed(roundsExecuted: var n):
        roundsExecuted += n;
        print("Optimization done after $roundsExecuted rounds");
        return;
      case OptimizationOutcome_Skipped():
        print("Optimization skipped — wallet already optimal");
        return;
    }
  }
  // ANCHOR_END: optimize-leaves-single-round
}

void handleAutoOptimizationEvent(AutoOptimizationEvent optimizationEvent) {
  // ANCHOR: auto-optimization-events
  switch (optimizationEvent) {
    case AutoOptimizationEvent_Started(totalRounds: var totalRounds):
      print("Auto-optimization started with $totalRounds rounds");
      break;
    case AutoOptimizationEvent_RoundCompleted(currentRound: var currentRound, totalRounds: var totalRounds):
      print("Auto-optimization round $currentRound of $totalRounds completed");
      break;
    case AutoOptimizationEvent_Completed():
      print("Auto-optimization completed successfully");
      break;
    case AutoOptimizationEvent_Cancelled():
      print("Auto-optimization was cancelled");
      break;
    case AutoOptimizationEvent_Failed(error: var error):
      print("Auto-optimization failed: $error");
      break;
    case AutoOptimizationEvent_Skipped():
      print("Auto-optimization was skipped because leaves are already optimal");
      break;
  }
  // ANCHOR_END: auto-optimization-events
}
