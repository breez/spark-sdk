import { type AutoOptimizationEvent, type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleOptimizeLeavesFull = async (sdk: BreezSdk) => {
  // ANCHOR: optimize-leaves-full
  const outcome = (await sdk.optimizeLeaves({ mode: 'full' })).outcome

  switch (outcome.type) {
    case 'completed': {
      if (outcome.roundsExecuted === 0) {
        console.log('Optimization skipped — wallet already optimal')
      } else {
        console.log(`Optimization completed in ${outcome.roundsExecuted} rounds`)
      }
      break
    }
    case 'inProgress': {
      // Full mode runs to completion in one call, so inProgress is
      // not reachable here.
      break
    }
  }
  // ANCHOR_END: optimize-leaves-full
}

const exampleOptimizeLeavesSingleRound = async (sdk: BreezSdk) => {
  // ANCHOR: optimize-leaves-single-round
  let roundsExecuted = 0
  while (true) {
    const outcome = (await sdk.optimizeLeaves({ mode: 'singleRound' })).outcome

    if (outcome.type === 'inProgress') {
      roundsExecuted += 1
      console.log(`Executed round ${roundsExecuted}`)
    } else if (outcome.type === 'completed') {
      roundsExecuted += outcome.roundsExecuted
      if (roundsExecuted === 0) {
        console.log('Optimization skipped — wallet already optimal')
      } else {
        console.log(`Optimization done after ${roundsExecuted} rounds`)
      }
      break
    }
  }
  // ANCHOR_END: optimize-leaves-single-round
}

const exampleAutoOptimizationEvents = async (event: AutoOptimizationEvent) => {
  // ANCHOR: auto-optimization-events
  switch (event.type) {
    case 'started': {
      console.log(`Auto-optimization started with ${event.totalRounds} rounds`)
      break
    }
    case 'roundCompleted': {
      console.log(`Auto-optimization round ${event.currentRound} of ${event.totalRounds} completed`)
      break
    }
    case 'completed': {
      console.log('Auto-optimization completed successfully')
      break
    }
    case 'cancelled': {
      console.log('Auto-optimization was cancelled')
      break
    }
    case 'failed': {
      console.log(`Auto-optimization failed: ${event.error}`)
      break
    }
    case 'skipped': {
      console.log('Auto-optimization was skipped because leaves are already optimal')
      break
    }
  }
  // ANCHOR_END: auto-optimization-events
}
