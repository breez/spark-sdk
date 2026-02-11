use bitcoin::hashes::{Hash, sha256};
use clap::Subcommand;
use qrcode_rs::{EcLevel, QrCode, render::unicode};
use rand::rngs::OsRng;
use spark_wallet::{InvoiceDescription, Preimage, SparkWallet};

#[derive(Clone, Debug, Subcommand)]
pub enum LightningCommand {
    /// Create a lightning invoice.
    CreateInvoice {
        amount_sat: u64,
        description: Option<String>,
        expiry_secs: Option<u32>,
    },
    /// Create a HODL lightning invoice (no preimage stored with operators).
    /// The preimage is generated locally and printed. Use `htlc claim` to settle later.
    CreateHodlInvoice {
        amount_sat: u64,
        description: Option<String>,
        expiry_secs: Option<u32>,
    },
    /// Fetch a lightning receive payment.
    FetchReceivePayment { id: String },
    /// Fetch a lightning send fee estimate.
    FetchSendFeeEstimate {
        invoice: String,
        amount_to_send: Option<u64>,
    },
    /// Fetch a lightning send payment.
    FetchSendPayment { id: String },
    /// Pay a lightning invoice.
    PayInvoice {
        #[arg(long)]
        invoice: String,
        #[arg(long)]
        max_fee_sat: Option<u64>,
        #[arg(long)]
        amount_to_send: Option<u64>,
        #[arg(
            long,
            default_value_t = true,
            help = "Prefer to pay to the spark address, default true"
        )]
        prefer_spark: bool,
    },
}

pub async fn handle_command(
    wallet: &SparkWallet,
    command: LightningCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        LightningCommand::CreateInvoice {
            amount_sat,
            description,
            expiry_secs,
        } => {
            let desc = description.map(InvoiceDescription::Memo);
            let payment = wallet
                .create_lightning_invoice(amount_sat, desc, None, expiry_secs, true)
                .await?;
            let qr = QrCode::with_error_correction_level(&payment.invoice, EcLevel::L)
                .unwrap()
                .render::<unicode::Dense1x2>()
                .dark_color(unicode::Dense1x2::Light)
                .light_color(unicode::Dense1x2::Dark)
                .max_dimensions(50, 50)
                .build();
            println!("{}\n\n{}", serde_json::to_string_pretty(&payment)?, qr);
        }
        LightningCommand::CreateHodlInvoice {
            amount_sat,
            description,
            expiry_secs,
        } => {
            // Generate preimage locally
            let preimage_secret = bitcoin::secp256k1::SecretKey::new(&mut OsRng);
            let preimage_bytes = preimage_secret.secret_bytes();
            let preimage = Preimage::try_from(preimage_bytes.to_vec())
                .map_err(|e| format!("Failed to create preimage: {e}"))?;
            let payment_hash = sha256::Hash::hash(&preimage_bytes);

            let desc = description.map(InvoiceDescription::Memo);
            let payment = wallet
                .create_hodl_lightning_invoice(amount_sat, desc, payment_hash, None, expiry_secs)
                .await?;

            let qr = QrCode::with_error_correction_level(&payment.invoice, EcLevel::L)
                .unwrap()
                .render::<unicode::Dense1x2>()
                .dark_color(unicode::Dense1x2::Light)
                .light_color(unicode::Dense1x2::Dark)
                .max_dimensions(50, 50)
                .build();

            println!("HODL Invoice created!");
            println!("Preimage (save this!): {}", preimage.encode_hex());
            println!("Payment hash: {}", payment_hash);
            println!("\n{}\n\n{}", serde_json::to_string_pretty(&payment)?, qr);
            println!(
                "\nTo settle after payment: htlc claim -p {}",
                preimage.encode_hex()
            );
        }
        LightningCommand::FetchReceivePayment { id } => {
            let payment = wallet.fetch_lightning_receive_payment(&id).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        LightningCommand::FetchSendFeeEstimate {
            invoice,
            amount_to_send,
        } => {
            let fee = wallet
                .fetch_lightning_send_fee_estimate(&invoice, amount_to_send)
                .await?;
            println!("{fee}");
        }
        LightningCommand::FetchSendPayment { id } => {
            let payment = wallet.fetch_lightning_send_payment(&id).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        LightningCommand::PayInvoice {
            invoice,
            max_fee_sat,
            amount_to_send,
            prefer_spark,
        } => {
            let payment = wallet
                .pay_lightning_invoice(&invoice, max_fee_sat, amount_to_send, prefer_spark, None)
                .await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
    }

    Ok(())
}
