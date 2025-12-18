use nostr::{event::EventBuilder, util::JsonUtil};

pub fn create_zap_receipt(
    zap_request: &str,
    invoice: &str,
    preimage: Option<String>,
) -> Result<EventBuilder, String> {
    // Parse the zap request event
    let zap_request_event = nostr::Event::from_json(zap_request)
        .map_err(|e| format!("Failed to parse zap request: {e}"))?;

    // Build the zap receipt event
    Ok(EventBuilder::zap_receipt(
        invoice,
        preimage,
        &zap_request_event,
    ))
}
