import logging
from breez_sdk_spark import (
    BreezSdk,
    AutoOptimizationEvent,
    OptimizationMode,
    OptimizationOutcome,
    OptimizeLeavesRequest,
)

async def run_full_optimization(sdk: BreezSdk):
    # ANCHOR: optimize-leaves-full
    response = await sdk.optimize_leaves(OptimizeLeavesRequest(mode=OptimizationMode.FULL))
    outcome = response.outcome

    if isinstance(outcome, OptimizationOutcome.COMPLETED):
        if outcome.rounds_executed == 0:
            logging.debug("Optimization skipped — wallet already optimal")
        else:
            logging.debug(f"Optimization completed in {outcome.rounds_executed} rounds")
    elif isinstance(outcome, OptimizationOutcome.IN_PROGRESS):
        raise AssertionError("Full mode never returns IN_PROGRESS")
    # ANCHOR_END: optimize-leaves-full

async def run_optimization_one_round_at_a_time(sdk: BreezSdk):
    # ANCHOR: optimize-leaves-single-round
    rounds_executed = 0
    while True:
        response = await sdk.optimize_leaves(
            OptimizeLeavesRequest(mode=OptimizationMode.SINGLE_ROUND)
        )
        outcome = response.outcome
        if isinstance(outcome, OptimizationOutcome.IN_PROGRESS):
            rounds_executed += 1
            logging.debug(f"Executed round {rounds_executed}")
        elif isinstance(outcome, OptimizationOutcome.COMPLETED):
            rounds_executed += outcome.rounds_executed
            if rounds_executed == 0:
                logging.debug("Optimization skipped — wallet already optimal")
            else:
                logging.debug(f"Optimization done after {rounds_executed} rounds")
            break
    # ANCHOR_END: optimize-leaves-single-round

def handle_auto_optimization_event(event: AutoOptimizationEvent):
    # ANCHOR: auto-optimization-events
    if isinstance(event, AutoOptimizationEvent.STARTED):
        logging.debug(f"Auto-optimization started with {event.total_rounds} rounds")
    elif isinstance(event, AutoOptimizationEvent.ROUND_COMPLETED):
        logging.debug(f"Auto-optimization round {event.current_round} of "
            f"{event.total_rounds} completed")
    elif isinstance(event, AutoOptimizationEvent.COMPLETED):
        logging.debug("Auto-optimization completed successfully")
    elif isinstance(event, AutoOptimizationEvent.CANCELLED):
        logging.debug("Auto-optimization was cancelled")
    elif isinstance(event, AutoOptimizationEvent.FAILED):
        logging.debug(f"Auto-optimization failed: {event.error}")
    elif isinstance(event, AutoOptimizationEvent.SKIPPED):
        logging.debug("Auto-optimization was skipped because leaves are already optimal")
    # ANCHOR_END: auto-optimization-events
