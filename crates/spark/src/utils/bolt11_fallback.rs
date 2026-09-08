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

use bitcoin::bech32::Fe32IterExt as _;
use lightning_invoice::{Bolt11Invoice, RawTaggedField};

use crate::address::SparkAddress;

/// BOLT11 tag `f`, the fallback on-chain address field.
const FALLBACK_ADDRESS_TAG: u8 = 9;
/// Fallback address version reserved for a Spark destination. Outside the range
/// of the witness versions the field is otherwise used for, so a Lightning
/// implementation that does not know Spark skips it as an unknown version.
const SPARK_FALLBACK_VERSION: u8 = 31;
const RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID: u64 = 17592187092992000001;

/// A Spark destination advertised by a BOLT11 invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparkFallback {
    pub address: SparkAddress,
    /// The destination as the invoice carried it. Forwarded verbatim when paying
    /// so the receiver sees back exactly what it minted, whatever encoding it
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
    extract_fallback_field(invoice).or_else(|| extract_route_hint_address(invoice))
}

fn extract_fallback_field(invoice: &Bolt11Invoice) -> Option<SparkFallback> {
    let raw = invoice.clone().into_signed_raw();
    for field in &raw.data.tagged_fields {
        let RawTaggedField::UnknownSemantics(data) = field else {
            continue;
        };
        // Tag, then two length characters, then the version the payload opens with.
        if data.len() <= 4
            || data[0].to_u8() != FALLBACK_ADDRESS_TAG
            || data[3].to_u8() != SPARK_FALLBACK_VERSION
        {
            continue;
        }
        let bytes: Vec<u8> = data[4..].iter().copied().fes_to_bytes().collect();
        let encoded = String::from_utf8(bytes).ok()?;
        let address = encoded.parse().ok()?;
        return Some(SparkFallback { address, encoded });
    }
    None
}

fn extract_route_hint_address(invoice: &Bolt11Invoice) -> Option<SparkFallback> {
    let network = invoice.network().try_into().ok()?;
    let hints = invoice.route_hints();
    let hop = hints
        .iter()
        .flat_map(|hint| hint.0.iter())
        .find(|hop| hop.short_channel_id == RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID)?;
    let address = SparkAddress::new(hop.src_node_id, network, None);
    let encoded = address.to_address_string().ok()?;
    Some(SparkFallback { address, encoded })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use macros::test_all;

    use super::*;

    /// A BOLT11 carrying a Spark invoice in the version-31 fallback field.
    const INVOICE_WITH_FALLBACK_INVOICE: &str = "lnbcrt10u1p57ljcepp5tj54la7l3lw0wn47pmu7m4ynewd9pxzswxxrj37nvhjujhqfz9gqsp5q6ewua5nmkqced2xrkcj7hz2wyxlaya29n3e0vt79fkntf6cv2nsxq9z0rgqnp4qtlyk6hxw5h4hrdfdkd4nh2rv0mwyyqvdtakr3dv6m4vvsmfshvg6rzjqgp0s738klwqef7yr8yu54vv3wfuk4psv46x5laf6l6v5x4lwwahvqqqqrusum7gtyqqqqqqqqqqqqqq9qfv9lwdcxzuntwf6rzur8wdehjct3dsens7t509a8qvmg0p48j7r4d4n8y6rjx5enxwfhwfknq7pn89e85et4x4585a3s8p4hvve48pc8xan6vah8xum3va485ut3v4kn2wfnw3ek2wrhxu68gct4v35rsamkv9h8z6njx4e8zemw0p3kgmf4w9u857n3waknwwf5w9nnxutc0guxwut4v3uhqar4wfmrwdrvxpk8qer98pkn2errwfnxgatdx568wenj0pnrymrvddhxwue5w4mhq7tndpekcdt2xa3hjuntdcekudtpxgc8ydt3vsu8gcfedfekcem2x4eh5vpedenrwvr3dcm8vvr6wumxkufnvvcxkmphxemnvum4wdexgwt68p4qcqzpudq5wdcxzuntd9h8vmmfvdjs9qyyssqkxvng3kw2rze8h774a3gd2nfhx2378t532ryrftjj59s26a6wtr8gc5knn2nl6cm33vv99wnt5202mp2s9n87jy4tkhctyjgflc6ywqqcdwv3x";
    /// A BOLT11 carrying a bare Spark address in the version-31 fallback field.
    const INVOICE_WITH_FALLBACK_ADDRESS: &str = "lnbcrt10u1p57lj4upp5qxa002jtss48lwgqrzqwhsr5388gv09k4tllfy9hlv78akygcmzssp5hkpfxyy7xrd7qwwc043cfjma9u4z5vasxuyrgqrh6wrvyx6xkevqxq9z0rgqnp4qtlyk6hxw5h4hrdfdkd4nh2rv0mwyyqvdtakr3dv6m4vvsmfshvg6rzjqf6plzwgkgyrrwdygdekj8w8frztu8k7dz2x9nefwyc70vergwrqeapyqr6zgqqqq8hxk2qqae4jsqyugqcqzpudqswfhh2ar9dp5kuarn9qyyssqn9tv97k2a24a45vkt3zvckjt80ph2luwjje5c8ymtf7qr3m4nkaqrqzuc9gzvjufxvvhk2lvzfxerccvtclakmyq43hfyfuwzgkx7hspfh5vzf";

    #[test_all]
    fn extracts_invoice_from_fallback_field() {
        let invoice = Bolt11Invoice::from_str(INVOICE_WITH_FALLBACK_INVOICE).unwrap();
        let expected = SparkAddress::from_str("sparkrt1pgssyaql38ytyzp3hxjyxumfrhr53397rm0x39rzeu5hzv08kv358psvzgnssqgjzqqem593tse8w74taudh8wvanqjr5rqgnxcdm5qxzzqwm794qg3qxz8gqudypturv74l0lpde8m5dcrfdum54wfrxf2llkngs4uwpyshsl5j7cyrkn3n5a20r5qd8ta9jslgj5sz09nf70qn6v0zw6kq3c0kl76w6susrd9z8j").unwrap();
        let extracted = extract_spark_fallback(&invoice).unwrap();
        assert_eq!(extracted.address, expected);
        assert!(extracted.is_invoice());
        assert_eq!(extracted.encoded.parse::<SparkAddress>().unwrap(), expected);
    }

    #[test_all]
    fn extracts_address_from_fallback_field() {
        let invoice = Bolt11Invoice::from_str(INVOICE_WITH_FALLBACK_ADDRESS).unwrap();
        let expected = SparkAddress::from_str(
            "sparkrt1pgssyaql38ytyzp3hxjyxumfrhr53397rm0x39rzeu5hzv08kv358psvs7ph8y",
        )
        .unwrap();
        let extracted = extract_spark_fallback(&invoice).unwrap();
        assert_eq!(extracted.address, expected);
        assert!(!extracted.is_invoice());
    }
}
