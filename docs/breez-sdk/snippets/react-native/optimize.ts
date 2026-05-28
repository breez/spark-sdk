import { type AutoOptimizationEvent, AutoOptimizationEvent_Tags, OptimizationMode, type OptimizationOutcome, OptimizationOutcome_Tags, type BreezSdk } from '@breeztech/breez-sdk-spark-react-native'

const exampleOptimizeLeavesFull = async (sdk: BreezSdk) => {
  // ANCHOR: optimize-leaves-full
  const outcome = (await sdk.optimizeLeaves({ mode: OptimizationMode.Full })).outcome

  if (outcome.tag === OptimizationOutcome_Tags.Completed) {
    if (outcome.inner.roundsExecuted === 0) {
      console.log('Optimization skipped — wallet already optimal')
    } else {
      console.log(`Optimization completed in ${outcome.inner.roundsExecuted} rounds`)
    }
  } else if (outcome.tag === OptimizationOutcome_Tags.InProgress) {
    // Full mode runs to completion in one call, so InProgress is
    // not reachable here.
  }
  // ANCHOR_END: optimize-leaves-full
}

const exampleOptimizeLeavesSingleRound = async (sdk: BreezSdk) => {
  // ANCHOR: optimize-leaves-single-round
  let roundsExecuted = 0
  while (true) {
    const outcome: OptimizationOutcome = (await sdk.optimizeLeaves({ mode: OptimizationMode.SingleRound })).outcome

    if (outcome.tag === OptimizationOutcome_Tags.InProgress) {
      roundsExecuted += 1
      console.log(`Executed round ${roundsExecuted}`)
    } else if (outcome.tag === OptimizationOutcome_Tags.Completed) {
      roundsExecuted += outcome.inner.roundsExecuted
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

const exampleAutoOptimizationEvents = async (optimizationEvent: AutoOptimizationEvent) => {
  // ANCHOR: auto-optimization-events
  if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Started) {
    console.log(`Auto-optimization started with ${optimizationEvent.inner.totalRounds} rounds`)
  } else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.RoundCompleted) {
    console.log(`Auto-optimization round ${optimizationEvent.inner.currentRound} of ${optimizationEvent.inner.totalRounds} completed`)
  } else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Completed) {
    console.log('Auto-optimization completed successfully')
  } else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Cancelled) {
    console.log('Auto-optimization was cancelled')
  } else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Failed) {
    console.log(`Auto-optimization failed: ${optimizationEvent.inner.error}`)
  } else if (optimizationEvent.tag === AutoOptimizationEvent_Tags.Skipped) {
    console.log('Auto-optimization was skipped because leaves are already optimal')
  }
  // ANCHOR_END: auto-optimization-events
}
