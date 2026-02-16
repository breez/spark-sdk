import logging
from breez_sdk_spark import BreezClient, LeafOptimizationEvent

async def start_optimization(client: BreezClient):
    # ANCHOR: start-optimization
    client.start_leaf_optimization()
    # ANCHOR_END: start-optimization

async def cancel_optimization(client: BreezClient):
    # ANCHOR: cancel-optimization
    await client.cancel_leaf_optimization()
    # ANCHOR_END: cancel-optimization

async def get_optimization_progress(client: BreezClient):
    # ANCHOR: get-optimization-progress
    progress = client.get_leaf_optimization_progress()

    logging.debug(f"Optimization is running: {progress.is_running}")
    logging.debug(f"Current round: {progress.current_round}")
    logging.debug(f"Total rounds: {progress.total_rounds}")
    # ANCHOR_END: get-optimization-progress

def optimization_events(optimization_event: LeafOptimizationEvent):
    # ANCHOR: optimization-events
    if isinstance(optimization_event, LeafOptimizationEvent.STARTED):
        logging.debug(f"Optimization started with {optimization_event.total_rounds} rounds")
    elif isinstance(optimization_event, LeafOptimizationEvent.ROUND_COMPLETED):
        logging.debug(f"Optimization round {optimization_event.current_round} of "
            f"{optimization_event.total_rounds} completed")
    elif isinstance(optimization_event, LeafOptimizationEvent.COMPLETED):
        logging.debug("Optimization completed successfully")
    elif isinstance(optimization_event, LeafOptimizationEvent.CANCELLED):
        logging.debug("Optimization was cancelled")
    elif isinstance(optimization_event, LeafOptimizationEvent.FAILED):
        logging.debug(f"Optimization failed: {optimization_event.error}")
    elif isinstance(optimization_event, LeafOptimizationEvent.SKIPPED):
        logging.debug("Optimization was skipped because leaves are already optimal")
    # ANCHOR_END: optimization-events
