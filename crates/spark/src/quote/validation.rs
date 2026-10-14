//! Checks a wallet runs on a quoted manifest before signing it: the signature
//! attests to these exact bytes, so a mismatch caught afterwards is one already
//! signed.

use std::collections::HashSet;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};

use super::QuoteError;
use crate::operator::rpc::spark::manifest_amount::Amount;
use crate::operator::rpc::spark::{ManifestAmount, TransferManifest};

const BPS_DENOMINATOR: u64 = 10_000;

/// Which quantity a quoted receive's requested amount pins. It is chosen when
/// the quote is requested and never recorded in the manifest, so it cannot be
/// read back from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReceiveAmountBasis {
    /// The amount is what the receiver keeps; the markup is added on top.
    Net,
    /// The amount is the invoice total; the markup comes out of it.
    Gross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestDenomination {
    /// Every edge is a fixed amount of sats.
    Sats,
    /// At least one edge is a rate, resolved against whatever the payer sends.
    Bps,
}

/// What a quoted send must fund for one receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendManifestBucket {
    pub receiver_identity_public_key: PublicKey,
    pub amount_sats: u64,
}

/// Decided by the edges alone, as the operators decide it: they never read
/// `fees`. Flat legs may sit beside rates in a rate manifest.
pub fn manifest_denomination(manifest: &TransferManifest) -> ManifestDenomination {
    if manifest
        .edges
        .iter()
        .any(|edge| matches!(amount_of(edge.amount.as_ref()), Some(Amount::Bps(_))))
    {
        ManifestDenomination::Bps
    } else {
        ManifestDenomination::Sats
    }
}

/// What the payer is invoiced. Also the amount the invoice request must name:
/// the SSP refuses a request naming anything but the manifest's own edge sum.
pub fn manifest_gross_sats(manifest: &TransferManifest) -> Result<u64, QuoteError> {
    sum_sats(
        manifest.edges.iter().map(|edge| edge.amount.as_ref()),
        "edges",
    )
}

/// Total markup carried by the manifest. Zero on an unattributed quote.
pub fn manifest_fee_sats(manifest: &TransferManifest) -> Result<u64, QuoteError> {
    sum_sats(manifest.fees.iter().map(|fee| fee.amount.as_ref()), "fees")
}

/// What `receiver` keeps. Edges are merged by receiver key, so a receiver who
/// is also a fee payee has that fee folded into their edge and it is netted
/// out here.
pub fn manifest_net_sats_for(
    manifest: &TransferManifest,
    receiver: &PublicKey,
) -> Result<u64, QuoteError> {
    let receiver = receiver.serialize();
    let credited = sum_sats(
        receiver_edge_amounts(manifest, &receiver)?,
        "the receiver's edges",
    )?;
    let fees_to_receiver = sum_sats(
        manifest
            .fees
            .iter()
            .filter(|fee| fee.receiver_identity_public_key == receiver)
            .map(|fee| fee.amount.as_ref()),
        "the fees crediting the receiver",
    )?;
    credited
        .checked_sub(fees_to_receiver)
        .ok_or_else(|| QuoteError::invalid("manifest leaves the receiver nothing"))
}

/// The markup a receive quote carries on top of what the receiver keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReceiveQuoteFee {
    /// Flat markup in sats.
    pub flat_sats: u64,
    /// Markup as basis points of what the payer sends, beyond `flat_sats`.
    /// Only an amountless invoice is priced as a rate.
    pub rate_bps: u32,
}

/// What `receiver` gives up to the fee payees under a quote that passed
/// [`validate_quoted_receive_manifest`].
pub fn receive_quote_fee(
    manifest: &TransferManifest,
    receiver: &PublicKey,
) -> Result<ReceiveQuoteFee, QuoteError> {
    match manifest_denomination(manifest) {
        ManifestDenomination::Sats => {
            let gross = manifest_gross_sats(manifest)?;
            let net = manifest_net_sats_for(manifest, receiver)?;
            Ok(ReceiveQuoteFee {
                flat_sats: gross.saturating_sub(net),
                rate_bps: 0,
            })
        }
        ManifestDenomination::Bps => {
            // Every flat edge is a fee: the receiver's own edge is the rate.
            let flat_sats = sum_in(
                false,
                manifest.edges.iter().map(|edge| edge.amount.as_ref()),
                "edges",
            )?;
            let rate_bps = BPS_DENOMINATOR.saturating_sub(receiver_rate_bps(manifest, receiver)?);
            Ok(ReceiveQuoteFee {
                flat_sats,
                rate_bps: u32::try_from(rate_bps).unwrap_or(u32::MAX),
            })
        }
    }
}

/// Refuses a receive quote whose amounts are not what was asked for.
///
/// Scoped to the receiver's own edge rather than the manifest total: a total
/// that balances says nothing about who is being paid. `attestor` is whoever
/// signs the quote, which is the receiver unless the receive is delegated.
/// `amount_sats` of zero is an amountless invoice, which only a rate manifest
/// on a gross basis can price.
pub fn validate_quoted_receive_manifest(
    manifest: &TransferManifest,
    amount_sats: u64,
    basis: ReceiveAmountBasis,
    receiver: &PublicKey,
    attestor: &PublicKey,
) -> Result<(), QuoteError> {
    let denomination = manifest_denomination(manifest);
    if (denomination == ManifestDenomination::Bps) != (amount_sats == 0) {
        return Err(QuoteError::invalid(format!(
            "manifest denomination does not match the requested invoice shape: \
             expected {}",
            if amount_sats == 0 {
                "a basis points manifest for an amountless invoice"
            } else {
                "a satoshis manifest for a fixed-amount invoice"
            }
        )));
    }

    if denomination == ManifestDenomination::Bps {
        return validate_rate_receive_manifest(manifest, basis, receiver, attestor);
    }

    let net = manifest_net_sats_for(manifest, receiver)?;
    let gross = manifest_gross_sats(manifest)?;
    let fees = manifest_fee_sats(manifest)?;
    assert_edges_are_fee_backed(manifest, receiver, attestor)?;

    // Covers what the edge sweep cannot see: a fee whose payee holds no edge is
    // declared but unfunded.
    if net.checked_add(fees) != Some(gross) {
        return Err(QuoteError::invalid(format!(
            "manifest does not conserve value across its edges and fees: \
             gross={gross}, net={net}, fees={fees}"
        )));
    }

    let pinned = match basis {
        ReceiveAmountBasis::Net => net,
        ReceiveAmountBasis::Gross => gross,
    };
    if pinned != amount_sats {
        return Err(QuoteError::invalid(format!(
            "manifest does not match the requested amount on a {basis:?} basis: \
             net={net}, gross={gross}, requested={amount_sats}"
        )));
    }

    if net == 0 {
        return Err(QuoteError::invalid("manifest leaves the receiver nothing"));
    }
    Ok(())
}

fn validate_rate_receive_manifest(
    manifest: &TransferManifest,
    basis: ReceiveAmountBasis,
    receiver: &PublicKey,
    attestor: &PublicKey,
) -> Result<(), QuoteError> {
    if basis != ReceiveAmountBasis::Gross {
        return Err(QuoteError::invalid(
            "an amountless quote has no net to pin: the basis must be gross",
        ));
    }
    // Ahead of the partition sum, so a quote paying the receiver nothing is
    // named as that rather than as arithmetic.
    let receiver_bps = receiver_rate_bps(manifest, receiver)?;
    let edge_bps = sum_bps(
        manifest.edges.iter().map(|edge| edge.amount.as_ref()),
        "edges",
    )?;
    if edge_bps != BPS_DENOMINATOR {
        return Err(QuoteError::invalid(format!(
            "manifest edges do not partition the invoice: they sum to {edge_bps} bps"
        )));
    }
    assert_edges_are_fee_backed(manifest, receiver, attestor)?;
    assert_every_fee_is_funded(manifest)?;
    if receiver_bps == 0 {
        return Err(QuoteError::invalid("manifest leaves the receiver nothing"));
    }
    Ok(())
}

/// What a quoted send must fund per receiver, read off the signed edges. The
/// operators enforce exact cover against them, so a locally derived split could
/// fund buckets the manifest does not describe. `fees` annotates an edge's
/// composition and carves nothing out of it.
pub fn manifest_send_buckets(
    manifest: &TransferManifest,
) -> Result<Vec<SendManifestBucket>, QuoteError> {
    let mut buckets: Vec<SendManifestBucket> = Vec::new();
    for (i, edge) in manifest.edges.iter().enumerate() {
        let sats = match amount_of(edge.amount.as_ref()) {
            Some(Amount::Sats(sats)) if *sats > 0 => *sats,
            // The hasher omits a zero amount from the digest, so such an edge
            // would be funded by leaves the sender's signature does not cover.
            Some(Amount::Sats(_)) => {
                return Err(QuoteError::invalid(format!(
                    "edges[{i}] is not a positive whole number of sats"
                )));
            }
            _ => return Err(QuoteError::invalid(format!("edges[{i}] is not in sats"))),
        };
        let receiver = PublicKey::from_slice(&edge.receiver_identity_public_key).map_err(|e| {
            QuoteError::invalid(format!("edges[{i}] receiver is not a public key: {e}"))
        })?;
        match buckets
            .iter_mut()
            .find(|bucket| bucket.receiver_identity_public_key == receiver)
        {
            // Refused by validate_quoted_send_manifest before funding. Summed
            // rather than dropped so a direct caller still funds every edge.
            Some(bucket) => {
                bucket.amount_sats = bucket.amount_sats.checked_add(sats).ok_or_else(|| {
                    QuoteError::invalid(format!("edges[{i}] overflows its receiver's bucket"))
                })?;
            }
            None => buckets.push(SendManifestBucket {
                receiver_identity_public_key: receiver,
                amount_sats: sats,
            }),
        }
    }
    Ok(buckets)
}

/// Refuses a send quote this wallet should not fund. The inverse of the receive
/// identity checks: the signer is the sole sender, so every edge must debit it
/// and none may credit it. Deliberately not the receive conservation check,
/// which no send manifest satisfies: on a send every edge, the SSP's included,
/// is funded by the sender.
pub fn validate_quoted_send_manifest(
    manifest: &TransferManifest,
    amount_sats: u64,
    sender: &PublicKey,
) -> Result<(), QuoteError> {
    // A send is quoted in sats even for a zero-amount invoice, because the
    // caller supplies the amount. A rate manifest is a receive-shaped quote.
    if manifest_denomination(manifest) != ManifestDenomination::Sats {
        return Err(QuoteError::invalid(
            "a send quote must be denominated in satoshis",
        ));
    }

    let sender = sender.serialize();
    let mut seen_receivers = HashSet::new();
    for (i, edge) in manifest.edges.iter().enumerate() {
        if edge.sender_identity_public_key != sender {
            return Err(QuoteError::invalid(format!(
                "edges[{i}] is sent by another wallet: {}",
                hex::encode(&edge.sender_identity_public_key)
            )));
        }
        if edge.receiver_identity_public_key == sender {
            return Err(QuoteError::invalid(format!(
                "edges[{i}] pays the sending wallet"
            )));
        }
        // The operators refuse a repeated sender/receiver pair, so funding one
        // would strand the leaves at their prepare step.
        if !seen_receivers.insert(edge.receiver_identity_public_key.as_slice()) {
            return Err(QuoteError::invalid(format!(
                "edges[{i}] repeats a receiver"
            )));
        }
    }

    for (i, fee) in manifest.fees.iter().enumerate() {
        if !seen_receivers.contains(fee.receiver_identity_public_key.as_slice()) {
            return Err(QuoteError::invalid(format!(
                "fees[{i}] credits a key holding no edge: {}",
                hex::encode(&fee.receiver_identity_public_key)
            )));
        }
    }

    // The payment and every fee ride inside the same edge sum, so the gross is
    // what the wallet parts with.
    let gross = manifest_gross_sats(manifest)?;
    if gross < amount_sats {
        return Err(QuoteError::invalid(format!(
            "manifest does not cover the payment amount: gross={gross}, amount={amount_sats}"
        )));
    }
    Ok(())
}

fn amount_of(amount: Option<&ManifestAmount>) -> Option<&Amount> {
    amount.and_then(|amount| amount.amount.as_ref())
}

/// Sums sats amounts. A rate is an error rather than zero: counting a bps
/// amount as absent is how a rate quote would slip through a fixed-amount
/// check.
fn sum_sats<'a>(
    amounts: impl IntoIterator<Item = Option<&'a ManifestAmount>>,
    context: &str,
) -> Result<u64, QuoteError> {
    amounts
        .into_iter()
        .enumerate()
        .try_fold(0u64, |total, (i, amount)| match amount_of(amount) {
            Some(Amount::Sats(sats)) => total
                .checked_add(*sats)
                .ok_or_else(|| QuoteError::invalid(format!("sum of {context} overflows"))),
            Some(Amount::Bps(_)) => Err(QuoteError::invalid(format!(
                "entry {i} among {context} is denominated in basis points, \
                 which a fixed-amount check cannot read"
            ))),
            None => Err(QuoteError::invalid(format!(
                "entry {i} among {context} has no amount"
            ))),
        })
}

/// Sums bps amounts, skipping flat legs.
fn sum_bps<'a>(
    amounts: impl IntoIterator<Item = Option<&'a ManifestAmount>>,
    context: &str,
) -> Result<u64, QuoteError> {
    amounts
        .into_iter()
        .enumerate()
        .try_fold(0u64, |total, (i, amount)| match amount_of(amount) {
            Some(Amount::Bps(bps)) => Ok(total.saturating_add(u64::from(*bps))),
            Some(Amount::Sats(_)) => Ok(total),
            None => Err(QuoteError::invalid(format!(
                "entry {i} among {context} has no amount"
            ))),
        })
}

/// The receiver's edge amounts, refusing anything but exactly one edge: two
/// edges to one key make "what the receiver is paid" ambiguous.
fn receiver_edge_amounts<'a>(
    manifest: &'a TransferManifest,
    receiver: &[u8; 33],
) -> Result<Vec<Option<&'a ManifestAmount>>, QuoteError> {
    let amounts: Vec<_> = manifest
        .edges
        .iter()
        .filter(|edge| edge.receiver_identity_public_key == receiver)
        .map(|edge| edge.amount.as_ref())
        .collect();
    if amounts.len() != 1 {
        return Err(QuoteError::invalid(format!(
            "manifest pays the receiver on {} edges",
            amounts.len()
        )));
    }
    Ok(amounts)
}

/// The rate the receiver keeps: their single edge, less any fee credited back
/// to the same key.
fn receiver_rate_bps(manifest: &TransferManifest, receiver: &PublicKey) -> Result<u64, QuoteError> {
    let receiver = receiver.serialize();
    let credited = sum_bps(
        receiver_edge_amounts(manifest, &receiver)?,
        "the receiver's edges",
    )?;
    let fees_to_receiver = sum_bps(
        manifest
            .fees
            .iter()
            .filter(|fee| fee.receiver_identity_public_key == receiver)
            .map(|fee| fee.amount.as_ref()),
        "the fees crediting the receiver",
    )?;
    Ok(credited.saturating_sub(fees_to_receiver))
}

/// Every edge that is not the receiver's must be exactly a declared fee, and no
/// edge may debit the receiver or the attestor. Without the first, an extra
/// edge to any key inflates what the payer is invoiced while every amount
/// comparison on the receiver's edge still matches. Without the second, the
/// attestation would state consent to a debit; under delegation the payee signs
/// nothing and cannot object.
fn assert_edges_are_fee_backed(
    manifest: &TransferManifest,
    receiver: &PublicKey,
    attestor: &PublicKey,
) -> Result<(), QuoteError> {
    let receiver = receiver.serialize();
    let attestor = attestor.serialize();
    let mut compared = HashSet::new();
    for (i, edge) in manifest.edges.iter().enumerate() {
        let debited = if edge.sender_identity_public_key == attestor {
            Some("attesting")
        } else if edge.sender_identity_public_key == receiver {
            Some("receiving")
        } else {
            None
        };
        if let Some(debited) = debited {
            return Err(QuoteError::invalid(format!(
                "edges[{i}] debits the {debited} wallet"
            )));
        }

        let payee = edge.receiver_identity_public_key.as_slice();
        if payee == receiver {
            continue;
        }

        // Compared in the edge's own denomination and summed across every edge
        // to this payee: a flat leg is charged off the top, so reading it as a
        // rate reads it as nothing, and the operators pay all of a payee's
        // edges, so one fee cannot cover two of them.
        let is_rate = is_rate(edge.amount.as_ref());
        if !compared.insert((payee, is_rate)) {
            continue;
        }
        let unit = if is_rate { "bps" } else { "sats" };
        let paid = sum_in(
            is_rate,
            manifest
                .edges
                .iter()
                .filter(|other| other.receiver_identity_public_key == payee)
                .map(|other| other.amount.as_ref()),
            "the edges to this payee",
        )?;
        let owed = sum_in(
            is_rate,
            manifest
                .fees
                .iter()
                .filter(|fee| fee.receiver_identity_public_key == payee)
                .map(|fee| fee.amount.as_ref()),
            "the fees to this payee",
        )?;
        if paid != owed {
            return Err(QuoteError::invalid(format!(
                "manifest pays {paid} {unit} to a key owed {owed} in fees: {}",
                hex::encode(payee)
            )));
        }
    }
    Ok(())
}

/// Every fee must be answered by an edge to the same key in the same
/// denomination. The sats path gets this from its conservation check; a rate
/// manifest has no total to conserve.
fn assert_every_fee_is_funded(manifest: &TransferManifest) -> Result<(), QuoteError> {
    for (i, fee) in manifest.fees.iter().enumerate() {
        let fee_is_rate = is_rate(fee.amount.as_ref());
        let funded = manifest.edges.iter().any(|edge| {
            edge.receiver_identity_public_key == fee.receiver_identity_public_key
                && is_rate(edge.amount.as_ref()) == fee_is_rate
        });
        if !funded {
            return Err(QuoteError::invalid(format!(
                "fees[{i}] credits a key holding no matching edge: {}",
                hex::encode(&fee.receiver_identity_public_key)
            )));
        }
    }
    Ok(())
}

fn is_rate(amount: Option<&ManifestAmount>) -> bool {
    matches!(amount_of(amount), Some(Amount::Bps(_)))
}

/// Sums the amounts of one denomination, ignoring the other.
fn sum_in<'a>(
    rate: bool,
    amounts: impl IntoIterator<Item = Option<&'a ManifestAmount>>,
    context: &str,
) -> Result<u64, QuoteError> {
    let same_denomination = amounts
        .into_iter()
        .filter(|amount| is_rate(*amount) == rate);
    if rate {
        sum_bps(same_denomination, context)
    } else {
        sum_sats(same_denomination, context)
    }
}
