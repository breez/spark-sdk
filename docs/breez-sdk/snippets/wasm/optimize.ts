import { type OptimizationEvent, type BreezSdk } from '@breeztech/breez-sdk-spark'

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

const exampleOptimizationEvents = async (event: OptimizationEvent) => {
  // ANCHOR: optimization-events
  switch (event.type) {
    case 'started': {
      console.log(`Optimization started with ${event.totalRounds} rounds`)
      break
    }
    case 'roundCompleted': {
      console.log(`Optimization round ${event.currentRound} of ${event.totalRounds} completed`)
      break
    }
    case 'completed': {
      console.log('Optimization completed successfully')
      break
    }
    case 'cancelled': {
      console.log('Optimization was cancelled')
      break
    }
    case 'failed': {
      console.log(`Optimization failed: ${event.error}`)
      break
    }
    case 'skipped': {
      console.log('Optimization was skipped because leaves are already optimal')
      break
    }
  }
  // ANCHOR_END: optimization-events
}
