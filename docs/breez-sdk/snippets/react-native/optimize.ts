import { type OptimizationEvent, OptimizationEvent_Tags, type BreezSdk } from '@breeztech/breez-sdk-spark-react-native'

const exampleStartOptimization = async (sdk: BreezSdk) => {
  // ANCHOR: start-optimization
  sdk.startLeafOptimization()
  // ANCHOR_END: start-optimization
}

const exampleCancelOptimization = async (sdk: BreezSdk) => {
  // ANCHOR: cancel-optimization
  await sdk.cancelLeafOptimization()
  // ANCHOR_END: cancel-optimization
}

const exampleGetOptimizationProgress = async (sdk: BreezSdk) => {
  // ANCHOR: get-optimization-progress
  const progress = sdk.getLeafOptimizationProgress()

  console.log(`Optimization is running: ${progress.isRunning}`)
  console.log(`Current round: ${progress.currentRound}`)
  console.log(`Total rounds: ${progress.totalRounds}`)
  // ANCHOR_END: get-optimization-progress
}

const exampleOptimizationEvents = async (optimizationEvent: OptimizationEvent) => {
  // ANCHOR: optimization-events
  if (optimizationEvent.tag === OptimizationEvent_Tags.Started) {
    console.log(`Optimization started with ${optimizationEvent.inner.totalRounds} rounds`)
  } else if (optimizationEvent.tag === OptimizationEvent_Tags.RoundCompleted) {
    console.log(`Optimization round ${optimizationEvent.inner.currentRound} of ${optimizationEvent.inner.totalRounds} completed`)
  } else if (optimizationEvent.tag === OptimizationEvent_Tags.Completed) {
    console.log('Optimization completed successfully')
  } else if (optimizationEvent.tag === OptimizationEvent_Tags.Cancelled) {
    console.log('Optimization was cancelled')
  } else if (optimizationEvent.tag === OptimizationEvent_Tags.Failed) {
    console.log(`Optimization failed: ${optimizationEvent.inner.error}`)
  } else if (optimizationEvent.tag === OptimizationEvent_Tags.Skipped) {
    console.log('Optimization was skipped because leaves are already optimal')
  }
  // ANCHOR_END: optimization-events
}
