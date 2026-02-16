import { type LeafOptimizationEvent, type BreezClient } from '@breeztech/breez-sdk-spark'

const exampleStartOptimization = async (client: BreezClient) => {
  // ANCHOR: start-optimization
  client.optimization.start()
  // ANCHOR_END: start-optimization
}

const exampleCancelOptimization = async (client: BreezClient) => {
  // ANCHOR: cancel-optimization
  await client.optimization.cancel()
  // ANCHOR_END: cancel-optimization
}

const exampleGetOptimizationProgress = async (client: BreezClient) => {
  // ANCHOR: get-optimization-progress
  const progress = client.optimization.progress

  console.log(`Optimization is running: ${progress.isRunning}`)
  console.log(`Current round: ${progress.currentRound}`)
  console.log(`Total rounds: ${progress.totalRounds}`)
  // ANCHOR_END: get-optimization-progress
}

const exampleOptimizationEvents = async (event: LeafOptimizationEvent) => {
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
