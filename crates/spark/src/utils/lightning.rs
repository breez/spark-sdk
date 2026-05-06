use bitcoin::bech32::Fe32IterExt as _;
use lightning_invoice::{Bolt11Invoice, RawTaggedField};

use crate::address::SparkAddress;

const RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID: u64 = 17592187092992000001;

pub fn extract_bolt11_spark_address(
    decoded_invoice: &Bolt11Invoice,
) -> Option<(
    /* address */ SparkAddress,
    /* address_str */ String,
)> {
    let Ok(network) = decoded_invoice.network().try_into() else {
        return None;
    };
    // MRH (legacy) - kept for backwards compatibility
    for route_hint in decoded_invoice.route_hints() {
        for node in route_hint.0 {
            if node.short_channel_id == RECEIVER_IDENTITY_PUBLIC_KEY_SHORT_CHANNEL_ID {
                let address = SparkAddress::new(node.src_node_id, network, None);
                let Ok(address_str) = address.to_address_string() else {
                    return None;
                };
                return Some((address, address_str));
            }
        }
    }
    // Fallback address - extract from version 31
    let raw_invoice = decoded_invoice.clone().into_signed_raw();
    for field in &raw_invoice.data.tagged_fields {
        if let RawTaggedField::UnknownSemantics(data) = field {
            let field_version = data[3].to_u8();
            if field_version == 31 {
                let bytes: Vec<u8> = data[4..].iter().copied().fes_to_bytes().collect();
                let Ok(spark_address_str) = String::from_utf8(bytes) else {
                    return None;
                };
                return spark_address_str
                    .parse::<SparkAddress>()
                    .ok()
                    .map(|addr| (addr, spark_address_str));
            }
        }
    }
    None
}

#[cfg(test)]
mod test {
    use std::str::FromStr as _;

    use lightning_invoice::Bolt11Invoice;
    use macros::test_all;

    use crate::{address::SparkAddress, utils::lightning::extract_bolt11_spark_address};

    #[test_all]
    fn test_bolt11_fallback_invoice() {
        let fallback_invoice = Bolt11Invoice::from_str("lnbcrt10u1p57ljcepp5tj54la7l3lw0wn47pmu7m4ynewd9pxzswxxrj37nvhjujhqfz9gqsp5q6ewua5nmkqced2xrkcj7hz2wyxlaya29n3e0vt79fkntf6cv2nsxq9z0rgqnp4qtlyk6hxw5h4hrdfdkd4nh2rv0mwyyqvdtakr3dv6m4vvsmfshvg6rzjqgp0s738klwqef7yr8yu54vv3wfuk4psv46x5laf6l6v5x4lwwahvqqqqrusum7gtyqqqqqqqqqqqqqq9qfv9lwdcxzuntwf6rzur8wdehjct3dsens7t509a8qvmg0p48j7r4d4n8y6rjx5enxwfhwfknq7pn89e85et4x4585a3s8p4hvve48pc8xan6vah8xum3va485ut3v4kn2wfnw3ek2wrhxu68gct4v35rsamkv9h8z6njx4e8zemw0p3kgmf4w9u857n3waknwwf5w9nnxutc0guxwut4v3uhqar4wfmrwdrvxpk8qer98pkn2errwfnxgatdx568wenj0pnrymrvddhxwue5w4mhq7tndpekcdt2xa3hjuntdcekudtpxgc8ydt3vsu8gcfedfekcem2x4eh5vpedenrwvr3dcm8vvr6wumxkufnvvcxkmphxemnvum4wdexgwt68p4qcqzpudq5wdcxzuntd9h8vmmfvdjs9qyyssqkxvng3kw2rze8h774a3gd2nfhx2378t532ryrftjj59s26a6wtr8gc5knn2nl6cm33vv99wnt5202mp2s9n87jy4tkhctyjgflc6ywqqcdwv3x").unwrap();
        let expected_invoice = SparkAddress::from_str("sparkrt1pgssyaql38ytyzp3hxjyxumfrhr53397rm0x39rzeu5hzv08kv358psvzgnssqgjzqqem593tse8w74taudh8wvanqjr5rqgnxcdm5qxzzqwm794qg3qxz8gqudypturv74l0lpde8m5dcrfdum54wfrxf2llkngs4uwpyshsl5j7cyrkn3n5a20r5qd8ta9jslgj5sz09nf70qn6v0zw6kq3c0kl76w6susrd9z8j").unwrap();
        assert_eq!(
            extract_bolt11_spark_address(&fallback_invoice).map(|(spark_address, _)| spark_address),
            Some(expected_invoice)
        );
    }

    #[test_all]
    fn test_bolt11_fallback_address() {
        let fallback_invoice = Bolt11Invoice::from_str("lnbcrt10u1p57lj4upp5qxa002jtss48lwgqrzqwhsr5388gv09k4tllfy9hlv78akygcmzssp5hkpfxyy7xrd7qwwc043cfjma9u4z5vasxuyrgqrh6wrvyx6xkevqxq9z0rgqnp4qtlyk6hxw5h4hrdfdkd4nh2rv0mwyyqvdtakr3dv6m4vvsmfshvg6rzjqf6plzwgkgyrrwdygdekj8w8frztu8k7dz2x9nefwyc70vergwrqeapyqr6zgqqqq8hxk2qqae4jsqyugqcqzpudqswfhh2ar9dp5kuarn9qyyssqn9tv97k2a24a45vkt3zvckjt80ph2luwjje5c8ymtf7qr3m4nkaqrqzuc9gzvjufxvvhk2lvzfxerccvtclakmyq43hfyfuwzgkx7hspfh5vzf").unwrap();
        let expected_address = SparkAddress::from_str(
            "sparkrt1pgssyaql38ytyzp3hxjyxumfrhr53397rm0x39rzeu5hzv08kv358psvs7ph8y",
        )
        .unwrap();
        assert_eq!(
            extract_bolt11_spark_address(&fallback_invoice).map(|(spark_address, _)| spark_address),
            Some(expected_address)
        );
    }
}
