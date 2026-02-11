use clap::Subcommand;
use qrcode_rs::{EcLevel, QrCode, render::unicode};
use spark_wallet::{InvoiceDescription, SparkWallet};

#[derive(Clone, Debug, Subcommand)]
pub enum LightningCommand {
    /// Create a lightning invoice.
    CreateInvoice {
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
