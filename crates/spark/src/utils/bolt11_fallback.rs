//! Reading and writing the Spark payment destination a BOLT11 invoice can carry.
//!
//! A Spark-aware payer that finds one settles the invoice with a Spark transfer
//! instead of routing over Lightning. Two encodings exist:
//!
//! - The `f` (fallback address) tagged field at version 31, holding a bech32m
//!   Spark address or invoice as UTF-8. An embedded invoice is what makes the
//!   resulting transfer attributable to the BOLT11 it settled.
//! - A route hint whose `short_channel_id` is a sentinel, with the receiver's
//!   identity key as `src_node_id`. Carries a bare address, so a payment over it
//!   cannot be tied back to the invoice. Read for invoices from older receivers,
//!   never written.

use bitcoin::bech32::{ByteIterExt as _, Fe32, Fe32IterExt as _};
use lightning_invoice::{Bolt11Invoice, RawTaggedField};

use crate::address::SparkAddress;

/// BOLT11 tag `f`, the fallback on-chain address field.
const FALLBACK_ADDRESS_TAG: u8 = 9;
/// Fallback address version reserved for a Spark destination. Outside the range
/// of the witness versions the field is otherwise used for, so a Lightning
/// implementation that does not know Spark skips it as an unknown version.
const SPARK_FALLBACK_VERSION: u8 = 31;
const RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID: u64 = 17592187092992000001;
/// The most 5-bit words a BOLT11 tagged field can hold: its length is written
/// in two of them.
const MAX_TAGGED_FIELD_WORDS: usize = 1023;

/// A Spark invoice too long to fit in a BOLT11 fallback field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparkFallbackTooLong;

impl std::fmt::Display for SparkFallbackTooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Spark invoice is too long to fit in a BOLT11 fallback field")
    }
}

impl std::error::Error for SparkFallbackTooLong {}

/// A Spark destination advertised by a BOLT11 invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparkFallback {
    pub address: SparkAddress,
    /// The destination as the invoice carried it. Forwarded verbatim when paying
    /// so the receiver sees back exactly what it created, whatever encoding it
    /// chose.
    pub encoded: String,
}

impl SparkFallback {
    pub fn is_invoice(&self) -> bool {
        self.address.is_invoice()
    }

    /// The receiver as a bare address, dropping any invoice fields.
    ///
    /// A transfer is addressed to an identity: which invoice it settles rides
    /// alongside it rather than in the destination.
    pub fn receiver_address(&self) -> SparkAddress {
        SparkAddress::new(self.address.identity_public_key, self.address.network, None)
    }
}

/// The Spark destination a BOLT11 invoice offers, if any.
///
/// Prefers the version-31 fallback field, which may hold an invoice, over the
/// legacy route hint, which can only hold a bare address.
pub fn extract_spark_fallback(invoice: &Bolt11Invoice) -> Option<SparkFallback> {
    fallback_fields(invoice)
        .into_iter()
        .next()
        .flatten()
        .or_else(|| route_hint_addresses(invoice).into_iter().next())
}

/// Every Spark destination a BOLT11 invoice offers, in either encoding. `None`
/// is a version-31 fallback field that does not hold a Spark address.
///
/// A payer may settle over any one of them, depending on which encodings it
/// reads, so a receiver checking the invoice has to check them all.
pub fn all_spark_fallbacks(invoice: &Bolt11Invoice) -> Vec<Option<SparkAddress>> {
    fallback_fields(invoice)
        .into_iter()
        .map(|field| field.map(|field| field.address))
        .chain(
            route_hint_addresses(invoice)
                .into_iter()
                .map(|hint| Some(hint.address)),
        )
        .collect()
}

/// The version-31 fallback fields, in order, each read as a Spark destination.
fn fallback_fields(invoice: &Bolt11Invoice) -> Vec<Option<SparkFallback>> {
    let raw = invoice.clone().into_signed_raw();
    raw.data
        .tagged_fields
        .iter()
        .filter_map(|field| {
            let RawTaggedField::UnknownSemantics(data) = field else {
                return None;
            };
            // Tag, then two length characters, then the version the payload opens with.
            if data.len() <= 4
                || data[0].to_u8() != FALLBACK_ADDRESS_TAG
                || data[3].to_u8() != SPARK_FALLBACK_VERSION
            {
                return None;
            }
            let bytes: Vec<u8> = data[4..].iter().copied().fes_to_bytes().collect();
            let encoded = String::from_utf8(bytes).ok();
            Some(encoded.and_then(|encoded| {
                let address = encoded.parse().ok()?;
                Some(SparkFallback { address, encoded })
            }))
        })
        .collect()
}

/// The version-31 fallback field carrying `spark_invoice`, for a BOLT11 that is
/// signed after it is added.
pub fn spark_invoice_fallback_field(
    spark_invoice: &str,
) -> Result<RawTaggedField, SparkFallbackTooLong> {
    let value: Vec<Fe32> = std::iter::once(fe32(SPARK_FALLBACK_VERSION.into()))
        .chain(spark_invoice.bytes().bytes_to_fes())
        .collect();
    let len = value.len();
    if len > MAX_TAGGED_FIELD_WORDS {
        return Err(SparkFallbackTooLong);
    }
    let mut field = Vec::with_capacity(3 + len);
    field.push(fe32(FALLBACK_ADDRESS_TAG.into()));
    field.push(fe32(len >> 5));
    field.push(fe32(len & 0x1f));
    field.extend(value);
    Ok(RawTaggedField::UnknownSemantics(field))
}

/// `value` as a 5-bit word. Only called with values below 32.
fn fe32(value: usize) -> Fe32 {
    u8::try_from(value)
        .ok()
        .and_then(|value| Fe32::try_from(value).ok())
        .expect("a value below 32 fits in a 5-bit word")
}

fn route_hint_addresses(invoice: &Bolt11Invoice) -> Vec<SparkFallback> {
    let Ok(network) = invoice.network().try_into() else {
        return Vec::new();
    };
    invoice
        .route_hints()
        .iter()
        .flat_map(|hint| hint.0.iter())
        .filter(|hop| hop.short_channel_id == RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID)
        .filter_map(|hop| {
            let address = SparkAddress::new(hop.src_node_id, network, None);
            let encoded = address.to_address_string().ok()?;
            Some(SparkFallback { address, encoded })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use macros::test_all;

    use super::*;

    /// A BOLT11 carrying a Spark invoice in the version-31 fallback field.
    const INVOICE_WITH_FALLBACK_INVOICE: &str = "lnbcrt10u1p57ljcepp5tj54la7l3lw0wn47pmu7m4ynewd9pxzswxxrj37nvhjujhqfz9gqsp5q6ewua5nmkqced2xrkcj7hz2wyxlaya29n3e0vt79fkntf6cv2nsxq9z0rgqnp4qtlyk6hxw5h4hrdfdkd4nh2rv0mwyyqvdtakr3dv6m4vvsmfshvg6rzjqgp0s738klwqef7yr8yu54vv3wfuk4psv46x5laf6l6v5x4lwwahvqqqqrusum7gtyqqqqqqqqqqqqqq9qfv9lwdcxzuntwf6rzur8wdehjct3dsens7t509a8qvmg0p48j7r4d4n8y6rjx5enxwfhwfknq7pn89e85et4x4585a3s8p4hvve48pc8xan6vah8xum3va485ut3v4kn2wfnw3ek2wrhxu68gct4v35rsamkv9h8z6njx4e8zemw0p3kgmf4w9u857n3waknwwf5w9nnxutc0guxwut4v3uhqar4wfmrwdrvxpk8qer98pkn2errwfnxgatdx568wenj0pnrymrvddhxwue5w4mhq7tndpekcdt2xa3hjuntdcekudtpxgc8ydt3vsu8gcfedfekcem2x4eh5vpedenrwvr3dcm8vvr6wumxkufnvvcxkmphxemnvum4wdexgwt68p4qcqzpudq5wdcxzuntd9h8vmmfvdjs9qyyssqkxvng3kw2rze8h774a3gd2nfhx2378t532ryrftjj59s26a6wtr8gc5knn2nl6cm33vv99wnt5202mp2s9n87jy4tkhctyjgflc6ywqqcdwv3x";
    /// A BOLT11 carrying a bare Spark address in the legacy route hint.
    const INVOICE_WITH_ROUTE_HINT_ADDRESS: &str = "lnbcrt10u1p57lj4upp5qxa002jtss48lwgqrzqwhsr5388gv09k4tllfy9hlv78akygcmzssp5hkpfxyy7xrd7qwwc043cfjma9u4z5vasxuyrgqrh6wrvyx6xkevqxq9z0rgqnp4qtlyk6hxw5h4hrdfdkd4nh2rv0mwyyqvdtakr3dv6m4vvsmfshvg6rzjqf6plzwgkgyrrwdygdekj8w8frztu8k7dz2x9nefwyc70vergwrqeapyqr6zgqqqq8hxk2qqae4jsqyugqcqzpudqswfhh2ar9dp5kuarn9qyyssqn9tv97k2a24a45vkt3zvckjt80ph2luwjje5c8ymtf7qr3m4nkaqrqzuc9gzvjufxvvhk2lvzfxerccvtclakmyq43hfyfuwzgkx7hspfh5vzf";

    #[test_all]
    fn extracts_invoice_from_fallback_field() {
        let invoice = Bolt11Invoice::from_str(INVOICE_WITH_FALLBACK_INVOICE).unwrap();
        let expected = SparkAddress::from_str("sparkrt1pgssyaql38ytyzp3hxjyxumfrhr53397rm0x39rzeu5hzv08kv358psvzgnssqgjzqqem593tse8w74taudh8wvanqjr5rqgnxcdm5qxzzqwm794qg3qxz8gqudypturv74l0lpde8m5dcrfdum54wfrxf2llkngs4uwpyshsl5j7cyrkn3n5a20r5qd8ta9jslgj5sz09nf70qn6v0zw6kq3c0kl76w6susrd9z8j").unwrap();
        let extracted = extract_spark_fallback(&invoice).unwrap();
        assert_eq!(extracted.address, expected);
        assert!(extracted.is_invoice());
        assert_eq!(extracted.encoded.parse::<SparkAddress>().unwrap(), expected);
    }

    /// The field inside an invoice Lightspark's SSP minted, so the encoding is
    /// checked against theirs and not only against this module's own reader.
    fn lightspark_fallback_field() -> RawTaggedField {
        let invoice = Bolt11Invoice::from_str(INVOICE_WITH_FALLBACK_INVOICE).unwrap();
        invoice
            .into_signed_raw()
            .raw_invoice()
            .data
            .tagged_fields
            .iter()
            .find(|field| {
                matches!(field, RawTaggedField::UnknownSemantics(data)
                    if data[0].to_u8() == FALLBACK_ADDRESS_TAG
                        && data[3].to_u8() == SPARK_FALLBACK_VERSION)
            })
            .cloned()
            .unwrap()
    }

    #[test_all]
    fn writes_the_fallback_field_lightspark_writes() {
        let invoice = Bolt11Invoice::from_str(INVOICE_WITH_FALLBACK_INVOICE).unwrap();
        let spark_invoice = extract_spark_fallback(&invoice).unwrap().encoded;
        assert_eq!(
            spark_invoice_fallback_field(&spark_invoice).unwrap(),
            lightspark_fallback_field()
        );
    }

    /// 638 bytes take 1021 words and the version one more, the most the length
    /// can express; a 639th byte needs two more.
    #[test_all]
    fn refuses_a_spark_invoice_the_field_cannot_hold() {
        assert!(spark_invoice_fallback_field(&"q".repeat(638)).is_ok());
        assert_eq!(
            spark_invoice_fallback_field(&"q".repeat(639)),
            Err(SparkFallbackTooLong)
        );
    }

    #[test_all]
    fn reads_back_the_spark_invoice_it_writes() {
        let invoice = Bolt11Invoice::from_str(INVOICE_WITH_FALLBACK_INVOICE).unwrap();
        let spark_invoice = extract_spark_fallback(&invoice).unwrap().encoded;
        let field = spark_invoice_fallback_field(&spark_invoice).unwrap();
        let RawTaggedField::UnknownSemantics(data) = field else {
            panic!("expected an unknown-semantics field");
        };
        let bytes: Vec<u8> = data[4..].iter().copied().fes_to_bytes().collect();
        assert_eq!(String::from_utf8(bytes).unwrap(), spark_invoice);
        assert_eq!(
            usize::from(data[1].to_u8()) * 32 + usize::from(data[2].to_u8()),
            data.len() - 3
        );
    }

    #[test_all]
    fn extracts_address_from_route_hint() {
        let invoice = Bolt11Invoice::from_str(INVOICE_WITH_ROUTE_HINT_ADDRESS).unwrap();
        let expected = SparkAddress::from_str(
            "sparkrt1pgssyaql38ytyzp3hxjyxumfrhr53397rm0x39rzeu5hzv08kv358psvs7ph8y",
        )
        .unwrap();
        let extracted = extract_spark_fallback(&invoice).unwrap();
        assert_eq!(extracted.address, expected);
        assert!(!extracted.is_invoice());
    }

    /// A BOLT11 offering a route-hint address for `hint_key`'s identity, plus
    /// `fields` signed in after it.
    fn invoice_with(hint_key: u8, fields: Vec<RawTaggedField>) -> Bolt11Invoice {
        use bitcoin::hashes::{Hash as _, sha256};
        use bitcoin::secp256k1::{Secp256k1, SecretKey};
        use lightning_invoice::{
            Currency, InvoiceBuilder, PaymentSecret, RouteHint, RouteHintHop, RoutingFees,
        };

        let secp = Secp256k1::new();
        let node_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let hint = RouteHint(vec![RouteHintHop {
            src_node_id: SecretKey::from_slice(&[hint_key; 32])
                .unwrap()
                .public_key(&secp),
            short_channel_id: RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID,
            fees: RoutingFees {
                base_msat: 0,
                proportional_millionths: 0,
            },
            cltv_expiry_delta: 0,
            htlc_minimum_msat: None,
            htlc_maximum_msat: None,
        }]);
        let invoice = InvoiceBuilder::new(Currency::Regtest)
            .description("test".to_string())
            .payment_hash(sha256::Hash::from_byte_array([0x02; 32]))
            .payment_secret(PaymentSecret([0x03; 32]))
            .duration_since_epoch(std::time::Duration::from_secs(1_700_000_000))
            .min_final_cltv_expiry_delta(144)
            .private_route(hint)
            .build_signed(|hash| secp.sign_ecdsa_recoverable(hash, &node_key))
            .unwrap();
        let (mut raw, _, _) = invoice.into_signed_raw().into_parts();
        raw.data.tagged_fields.extend(fields);
        let signed = raw
            .sign(|hash| Ok::<_, ()>(secp.sign_ecdsa_recoverable(hash, &node_key)))
            .unwrap();
        Bolt11Invoice::from_signed(signed).unwrap()
    }

    /// A payer reading only route hints pays the address there, so a check of
    /// the preferred destination alone would miss it.
    #[test_all]
    fn lists_every_destination_in_either_encoding() {
        let spark_invoice = extract_spark_fallback(
            &Bolt11Invoice::from_str(INVOICE_WITH_FALLBACK_INVOICE).unwrap(),
        )
        .unwrap();
        let invoice = invoice_with(
            0x04,
            vec![
                spark_invoice_fallback_field(&spark_invoice.encoded).unwrap(),
                spark_invoice_fallback_field("not a spark address").unwrap(),
            ],
        );

        let hinted = SparkAddress::new(
            bitcoin::secp256k1::SecretKey::from_slice(&[0x04; 32])
                .unwrap()
                .public_key(&bitcoin::secp256k1::Secp256k1::new()),
            crate::Network::Regtest,
            None,
        );
        assert_eq!(
            all_spark_fallbacks(&invoice),
            vec![Some(spark_invoice.address.clone()), None, Some(hinted)]
        );
        assert_eq!(extract_spark_fallback(&invoice), Some(spark_invoice));
    }
}
