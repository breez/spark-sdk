import { type LeafOptimizationEvent, LeafOptimizationEvent_Tags, type BreezClient } from '@breeztech/breez-sdk-spark-react-native'

const exampleStartOptimization = async (client: BreezClient) => {
  // ANCHOR: start-optimization
  client.optimization().start()
  // ANCHOR_END: start-optimization
}

const exampleCancelOptimization = async (client: BreezClient) => {
  // ANCHOR: cancel-optimization
  await client.optimization().cancel()
  // ANCHOR_END: cancel-optimization
}

const exampleGetOptimizationProgress = async (client: BreezClient) => {
  // ANCHOR: get-optimization-progress
  const progress = client.optimization().progress()

  console.log(`Optimization is running: ${progress.isRunning}`)
  console.log(`Current round: ${progress.currentRound}`)
  console.log(`Total rounds: ${progress.totalRounds}`)
  // ANCHOR_END: get-optimization-progress
}

const exampleOptimizationEvents = async (optimizationEvent: LeafOptimizationEvent) => {
  // ANCHOR: optimization-events
  if (optimizationEvent.tag === LeafOptimizationEvent_Tags.Started) {
    console.log(`Optimization started with ${optimizationEvent.inner.totalRounds} rounds`)
  } else if (optimizationEvent.tag === LeafOptimizationEvent_Tags.RoundCompleted) {
    console.log(`Optimization round ${optimizationEvent.inner.currentRound} of ${optimizationEvent.inner.totalRounds} completed`)
  } else if (optimizationEvent.tag === LeafOptimizationEvent_Tags.Completed) {
    console.log('Optimization completed successfully')
  } else if (optimizationEvent.tag === LeafOptimizationEvent_Tags.Cancelled) {
    console.log('Optimization was cancelled')
  } else if (optimizationEvent.tag === LeafOptimizationEvent_Tags.Failed) {
    console.log(`Optimization failed: ${optimizationEvent.inner.error}`)
  } else if (optimizationEvent.tag === LeafOptimizationEvent_Tags.Skipped) {
    console.log('Optimization was skipped because leaves are already optimal')
  }
  // ANCHOR_END: optimization-events
}
