use nostr::{
    event::{Event, EventBuilder},
    util::JsonUtil,
};

pub fn create_zap_receipt(
    zap_request: &str,
    invoice: &str,
    preimage: Option<String>,
    signing_keys: &nostr::Keys,
) -> Result<Event, String> {
    // Parse the zap request event
    let zap_request_event = nostr::Event::from_json(zap_request)
        .map_err(|e| format!("Failed to parse zap request: {e}"))?;

    // Build and sign the zap receipt event
    EventBuilder::zap_receipt(invoice, preimage, &zap_request_event)
        .sign_with_keys(signing_keys)
        .map_err(|e| format!("Failed to build zap receipt: {e}"))
}
