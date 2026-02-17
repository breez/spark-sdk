use breez_sdk_common::input::{LocalInputType, parse_local};

use crate::SdkError;

/// Validates contact input, returns trimmed name on success
pub fn validate_contact_input(name: &str, payment_identifier: &str) -> Result<String, SdkError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(SdkError::InvalidInput(
            "Contact name cannot be empty".to_string(),
        ));
    }
    if name.len() > 100 {
        return Err(SdkError::InvalidInput(
            "Contact name cannot exceed 100 characters".to_string(),
        ));
    }
    let payment_identifier = payment_identifier.trim();
    if payment_identifier.is_empty() {
        return Err(SdkError::InvalidInput(
            "Payment identifier cannot be empty".to_string(),
        ));
    }

    validate_payment_identifier(payment_identifier)?;

    Ok(name)
}

fn validate_payment_identifier(payment_identifier: &str) -> Result<(), SdkError> {
    let parsed = parse_local(payment_identifier).map_err(|_| {
        SdkError::InvalidInput("Payment identifier is not a recognized payment format".to_string())
    })?;

    match parsed {
        LocalInputType::BitcoinAddress(_)
        | LocalInputType::SparkAddress(_)
        | LocalInputType::LightningAddress { .. }
        | LocalInputType::Lnurl { .. } => Ok(()),

        LocalInputType::Bolt11Invoice(_) => Err(SdkError::InvalidInput(
            "Bolt11 invoices are not reusable and cannot be used as a contact payment identifier"
                .to_string(),
        )),
        LocalInputType::Bip21(_) => Err(SdkError::InvalidInput(
            "BIP-21 URIs are not yet supported as a contact payment identifier".to_string(),
        )),
        LocalInputType::Bolt12InvoiceRequest(_) => Err(SdkError::InvalidInput(
            "Bolt12 invoice requests are not reusable and cannot be used as a contact payment identifier"
                .to_string(),
        )),
        LocalInputType::Bolt12Offer(_) => Err(SdkError::InvalidInput(
            "Bolt12 offers are not yet supported as a contact payment identifier".to_string(),
        )),
        LocalInputType::SparkInvoice(_) => Err(SdkError::InvalidInput(
            "Spark invoices are not reusable and cannot be used as a contact payment identifier"
                .to_string(),
        )),
        LocalInputType::SilentPaymentAddress(_) => Err(SdkError::InvalidInput(
            "Silent payment addresses are not yet supported as a contact payment identifier"
                .to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_NAME: &str = "Alice";

    #[test]
    fn test_accepts_lightning_address() {
        assert!(validate_contact_input(VALID_NAME, "user@domain.com").is_ok());
    }

    #[test]
    fn test_accepts_bitcoin_address() {
        assert!(validate_contact_input(VALID_NAME, "1andreas3batLhQa2FawWjeyjCqyBzypd").is_ok());
    }

    #[test]
    fn test_accepts_spark_address() {
        assert!(
            validate_contact_input(
                VALID_NAME,
                "sparkrt1pgssyuuuhnrrdjswal5c3s3rafw9w3y5dd4cjy3duxlf7hjzkp0rqx6dc0nltx"
            )
            .is_ok()
        );
    }

    #[test]
    fn test_accepts_lnurl() {
        assert!(validate_contact_input(
            VALID_NAME,
            "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf"
        )
        .is_ok());
    }

    #[test]
    fn test_rejects_bolt11_invoice() {
        let bolt11 = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";
        let result = validate_contact_input(VALID_NAME, bolt11);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not reusable"));
    }

    #[test]
    fn test_rejects_bolt12_offer() {
        let offer = "lno1zcss9mk8y3wkklfvevcrszlmu23kfrxh49px20665dqwmn4p72pksese";
        let result = validate_contact_input(VALID_NAME, offer);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not yet supported")
        );
    }

    #[test]
    fn test_rejects_garbage() {
        let result = validate_contact_input(VALID_NAME, "not_a_payment_format");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a recognized"));
    }

    #[test]
    fn test_rejects_empty_name() {
        assert!(validate_contact_input("", "user@domain.com").is_err());
    }

    #[test]
    fn test_rejects_empty_payment_identifier() {
        assert!(validate_contact_input(VALID_NAME, "").is_err());
    }
}
