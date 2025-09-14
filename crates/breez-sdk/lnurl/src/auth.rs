use axum::{
    extract::{self, Request},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use tracing::{debug, error};
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::state::State;

pub async fn auth<DB>(
    extract::State(state): extract::State<State<DB>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode>
where
    DB: Send + Sync + 'static,
{
    let Some(ca_cert) = state.ca_cert.as_ref() else {
        return Ok(next.run(req).await);
    };

    let Ok((_, ca_cert)) = X509Certificate::from_der(ca_cert.as_ref()) else {
        error!("Failed to parse CA certificate");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|header| header.to_str().ok());

    let Some(auth_header) = auth_header else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let Ok(cert_bytes) = BASE64.decode(auth_header.trim_start_matches("Bearer ").trim()) else {
        debug!("Failed to decode base64 certificate");
        return Err(StatusCode::UNAUTHORIZED);
    };

    let (_, cert) = X509Certificate::from_der(&cert_bytes).map_err(|e| {
        debug!("Failed to parse client certificate: {}", e);
        StatusCode::UNAUTHORIZED
    })?;

    if let Err(e) = verify_cert_against_ca(&cert, &ca_cert) {
        debug!("Failed to verify client certificate: {}", e);
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

fn verify_cert_against_ca(
    cert: &X509Certificate,
    ca: &X509Certificate,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check that cert.issuer == ca.subject
    if cert.issuer() != ca.subject() {
        return Err("issuer does not match CA subject".into());
    }

    // Verify the signature on `cert` using the CA public key
    cert.verify_signature(Some(ca.public_key()))
        .map_err(|e| format!("signature verification failed: {e:?}"))?;

    Ok(())
}
