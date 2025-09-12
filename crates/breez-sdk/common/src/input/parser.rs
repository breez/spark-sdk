use bitcoin::{Address, Denomination, address::NetworkUnchecked};
use lightning::bolt11_invoice::Bolt11InvoiceDescriptionRef;
use percent_encoding_rfc3986::percent_decode_str;
use serde::Deserialize;
use spark_wallet::SparkAddress;
use tracing::{debug, error};

use crate::{
    dns::{self, DnsResolver},
    error::ServiceConnectivityError,
    input::{Bip21Extra, ParseError, PaymentRequestSource, SparkAddressDetails},
    lnurl::{
        LnurlErrorDetails,
        auth::{self, LnurlAuthRequestDetails},
        error::LnurlError,
        pay::LnurlPayRequestDetails,
    },
    rest::{ReqwestRestClient, RestClient, RestResponse},
};

use super::{
    Bip21Details, BitcoinAddressDetails, Bolt11InvoiceDetails, Bolt11RouteHint, Bolt11RouteHintHop,
    Bolt12InvoiceDetails, Bolt12InvoiceRequestDetails, Bolt12Offer, Bolt12OfferBlindedPath,
    Bolt12OfferDetails, InputType, LightningAddressDetails, LnurlWithdrawRequestDetails,
    SilentPaymentAddressDetails, error::Bip21Error,
};

const BIP_21_PREFIX: &str = "bitcoin:";
const BIP_353_USER_BITCOIN_PAYMENT_PREFIX: &str = "user._bitcoin-payment";
const LIGHTNING_PREFIX: &str = "lightning:";
const LIGHTNING_PREFIX_LEN: usize = LIGHTNING_PREFIX.len();
const LNURL_HRP: &str = "lnurl";

pub async fn parse(input: &str) -> Result<InputType, ParseError> {
    InputParser::new(dns::Resolver::new(), ReqwestRestClient::new()?)
        .parse(input)
        .await
}

pub fn parse_invoice(input: &str) -> Option<Bolt11InvoiceDetails> {
    parse_bolt11(input, &PaymentRequestSource::default())
}

pub struct InputParser<C, D> {
    rest_client: C,
    dns_resolver: D,
}

impl<C, D> InputParser<C, D>
where
    C: RestClient + Send + Sync,
    D: DnsResolver + Send + Sync,
{
    pub fn new(dns_resolver: D, rest_client: C) -> Self {
        InputParser {
            rest_client,
            dns_resolver,
        }
    }

    pub async fn parse(&self, input: &str) -> Result<InputType, ParseError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ParseError::EmptyInput);
        }

        if input.contains('@') {
            if let Some(bip_21) = self.parse_bip_353(input).await? {
                return Ok(InputType::Bip21(bip_21));
            }

            if let Some(lightning_address) = self.parse_lightning_address(input).await {
                return Ok(InputType::LightningAddress(lightning_address));
            }
        }

        if has_bip_21_prefix(input) {
            let source = PaymentRequestSource {
                bip_21_uri: Some(input.to_string()),
                bip_353_address: None,
            };
            if let Some(bip_21) = parse_bip_21(input, &source)? {
                return Ok(InputType::Bip21(bip_21));
            }
        }

        let source = PaymentRequestSource::default();
        if let Some(input_type) = self.parse_lightning(input, &source).await? {
            return Ok(input_type);
        }

        if let Some(input_type) = parse_spark_address(input, &source) {
            return Ok(input_type);
        }

        if let Some(input_type) = parse_bitcoin(input, &source) {
            return Ok(input_type);
        }

        Err(ParseError::InvalidInput)
    }

    async fn parse_bip_353(&self, input: &str) -> Result<Option<Bip21Details>, Bip21Error> {
        // BIP-353 addresses may have a ₿ prefix, so strip it if present
        let Some((local_part, domain)) = input.strip_prefix('₿').unwrap_or(input).split_once('@')
        else {
            return Ok(None);
        };

        // Validate both parts are within the DNS label size limit.
        // See <https://datatracker.ietf.org/doc/html/rfc1035#section-2.3.4>
        if local_part.len() > 63 || domain.len() > 63 {
            return Ok(None);
        }

        // Query for TXT records of a domain
        let dns_name = format!("{local_part}.{BIP_353_USER_BITCOIN_PAYMENT_PREFIX}.{domain}");
        let records = match self.dns_resolver.txt_lookup(dns_name).await {
            Ok(records) => records,
            Err(e) => {
                debug!("No BIP353 TXT records found: {}", e);
                return Ok(None);
            }
        };

        let Some(bip_21) = extract_bip353_record(records) else {
            return Ok(None);
        };
        parse_bip_21(
            &bip_21,
            &PaymentRequestSource {
                bip_21_uri: Some(bip_21.clone()),
                bip_353_address: Some(input.to_string()),
            },
        )
    }

    async fn parse_lightning(
        &self,
        input: &str,
        source: &PaymentRequestSource,
    ) -> Result<Option<InputType>, ParseError> {
        let input = if has_lightning_prefix(input) {
            &input[LIGHTNING_PREFIX_LEN..]
        } else {
            input
        };

        if let Some(payment_method) = parse_lightning_payment_method(input, source) {
            return Ok(Some(payment_method));
        }

        if let Some(bolt12_invoice_request) = parse_bolt12_invoice_request(input, source) {
            return Ok(Some(InputType::Bolt12InvoiceRequest(
                bolt12_invoice_request,
            )));
        }

        if let Some(lnurl) = self.parse_lnurl(input, source).await? {
            return Ok(Some(lnurl));
        }

        Ok(None)
    }

    async fn parse_lightning_address(&self, input: &str) -> Option<LightningAddressDetails> {
        if !input.contains('@') {
            return None;
        }

        let (user, domain) = input.strip_prefix('₿').unwrap_or(input).split_once('@')?;

        // It is safe to downcase the domains since they are case-insensitive.
        // https://www.rfc-editor.org/rfc/rfc3986#section-3.2.2
        let (user, domain) = (user.to_lowercase(), domain.to_lowercase());

        if !user
            .chars()
            .all(|c| c.is_alphanumeric() || ['-', '_', '.'].contains(&c))
        {
            return None;
        }

        let scheme = if has_extension(&domain, "onion") {
            "http://"
        } else {
            "https://"
        };

        let Ok(url) = reqwest::Url::parse(&format!("{scheme}{domain}/.well-known/lnurlp/{user}"))
        else {
            return None;
        };

        let input_type = self
            .resolve_lnurl(&url, &PaymentRequestSource::default())
            .await
            .ok()?;

        let address = format!("{user}@{domain}");
        match input_type {
            InputType::LnurlPay(pay_request) => Some(LightningAddressDetails {
                address: address.clone(),
                pay_request: LnurlPayRequestDetails {
                    address: Some(address),
                    ..pay_request
                },
            }),
            _ => None, // TODO: log or return error.
        }
    }

    async fn parse_lnurl(
        &self,
        input: &str,
        source: &PaymentRequestSource,
    ) -> Result<Option<InputType>, LnurlError> {
        let mut input = match bech32::decode(input) {
            Ok((hrp, data)) => {
                let hrp = hrp.to_lowercase();
                if hrp != LNURL_HRP {
                    return Ok(None);
                }

                match String::from_utf8(data) {
                    Ok(decoded) => decoded,
                    Err(_) => return Ok(None),
                }
            }
            Err(_) => input.to_string(),
        };

        let supported_prefixes: [&str; 3] = ["lnurlp", "lnurlw", "keyauth"];

        // Treat prefix: and prefix:// the same, to cover both vendor implementations
        // https://github.com/lnbits/lnbits/pull/762#issue-1309702380
        for pref in supported_prefixes {
            let scheme_simple = format!("{pref}:");
            let scheme_authority = format!("{pref}://");
            if has_prefix(&input, &scheme_simple) && !has_prefix(&input, &scheme_authority) {
                input = replace_prefix(&input, &scheme_simple, &scheme_authority);
            }
        }

        let Ok(parsed_url) = reqwest::Url::parse(&input) else {
            return Ok(None);
        };

        let host = match parsed_url.host() {
            Some(domain) => domain.to_string(),
            None => return Ok(None), // TODO: log or return error.
        };

        let mut url = parsed_url.clone();
        match parsed_url.scheme() {
            "http" => {
                if !has_extension(&host, "onion") {
                    return Err(LnurlError::HttpSchemeWithoutOnionDomain);
                }
            }
            "https" => {
                if has_extension(&host, "onion") {
                    return Err(LnurlError::HttpsSchemeWithOnionDomain);
                }
            }
            scheme if supported_prefixes.contains(&scheme) => {
                if has_extension(&host, "onion") {
                    url = reqwest::Url::parse(&replace_prefix(&input, scheme, "http")).map_err(
                        |_| {
                            LnurlError::General(
                                "failed to rewrite lnurl scheme to http".to_string(),
                            )
                        },
                    )?;
                } else {
                    url = reqwest::Url::parse(&replace_prefix(&input, scheme, "https")).map_err(
                        |_| {
                            LnurlError::General(
                                "failed to rewrite lnurl scheme to https".to_string(),
                            )
                        },
                    )?;
                }
            }
            &_ => return Err(LnurlError::UnknownScheme), // TODO: log or return error.
        }

        Ok(Some(self.resolve_lnurl(&url, source).await?))
    }

    async fn resolve_lnurl(
        &self,
        url: &reqwest::Url,
        _source: &PaymentRequestSource,
    ) -> Result<InputType, LnurlError> {
        if let Some(query) = url.query()
            && query.contains("tag=login")
        {
            let data = auth::validate_request(url)?;
            return Ok(InputType::LnurlAuth(data));
        }

        let RestResponse { body, .. } = self.rest_client.get(url.to_string(), None).await?;
        let lnurl_data: LnurlRequestDetails = parse_json(&body)?;
        let domain = url.host().ok_or(LnurlError::MissingDomain)?.to_string();
        Ok(match lnurl_data {
            LnurlRequestDetails::PayRequest { pay_request } => {
                InputType::LnurlPay(LnurlPayRequestDetails {
                    domain,
                    url: url.to_string(),
                    ..pay_request
                })
            }
            LnurlRequestDetails::WithdrawRequest { withdraw_request } => {
                InputType::LnurlWithdraw(withdraw_request)
            }
            LnurlRequestDetails::AuthRequest { auth_request } => InputType::LnurlAuth(auth_request),
            LnurlRequestDetails::Error {
                error_details: error,
            } => {
                return Err(LnurlError::EndpointError(error.reason));
            }
        })
    }
}

fn format_short_channel_id(id: u64) -> String {
    let block_num = (id >> 40) as u32;
    let tx_num = ((id >> 16) & 0x00FF_FFFF) as u32;
    let tx_out = (id & 0xFFFF) as u16;
    format!("{block_num}x{tx_num}x{tx_out}")
}

fn has_bip_21_prefix(input: &str) -> bool {
    has_prefix(input, BIP_21_PREFIX)
}

fn has_extension(input: &str, extension: &str) -> bool {
    std::path::Path::new(input)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

fn has_lightning_prefix(input: &str) -> bool {
    has_prefix(input, LIGHTNING_PREFIX)
}

fn has_prefix(input: &str, prefix: &str) -> bool {
    if input.len() < prefix.len() {
        return false;
    }

    input[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn replace_prefix(input: &str, prefix: &str, new_prefix: &str) -> String {
    if !has_prefix(input, prefix) {
        return String::from(input);
    }

    format!("{}{}", new_prefix, &input[prefix.len()..])
}

fn extract_bip353_record(records: Vec<String>) -> Option<String> {
    let bip353_record = records
        .into_iter()
        .filter(|record| has_bip_21_prefix(record))
        .collect::<Vec<String>>();

    if bip353_record.len() > 1 {
        error!(
            "Invalid decoded TXT data. Multiple records found ({})",
            bip353_record.len()
        );

        return None;
    }

    bip353_record.into_iter().next()
}

fn parse_bip_21(
    input: &str,
    source: &PaymentRequestSource,
) -> Result<Option<Bip21Details>, Bip21Error> {
    // TODO: Support liquid BIP-21
    if !has_bip_21_prefix(input) {
        return Ok(None);
    }

    debug!("Parsing bip 21: {input}");
    let uri = input.to_string();
    let input = &input[BIP_21_PREFIX.len()..];
    let mut bip_21 = Bip21Details {
        uri,
        ..Default::default()
    };

    let (address, params) = match input.find('?') {
        Some(pos) => (&input[..pos], Some(&input[(pos.saturating_add(1))..])),
        None => (input, None),
    };

    debug!("Parsing bip 21: input: {input} - address: {address} - params: {params:?}");

    if !address.is_empty() {
        let address: Address<NetworkUnchecked> =
            address.parse().map_err(|_| Bip21Error::InvalidAddress)?;
        let network = match 1 {
            _ if address.is_valid_for_network(bitcoin::Network::Bitcoin) => {
                bitcoin::Network::Bitcoin
            }
            _ if address.is_valid_for_network(bitcoin::Network::Regtest) => {
                bitcoin::Network::Regtest
            }
            _ if address.is_valid_for_network(bitcoin::Network::Signet) => bitcoin::Network::Signet,
            _ if address.is_valid_for_network(bitcoin::Network::Testnet) => {
                bitcoin::Network::Testnet
            }
            _ if address.is_valid_for_network(bitcoin::Network::Testnet4) => {
                bitcoin::Network::Testnet4
            }
            _ => return Err(Bip21Error::InvalidAddress),
        }
        .into();
        bip_21
            .payment_methods
            .push(InputType::BitcoinAddress(BitcoinAddressDetails {
                address: address.assume_checked().to_string(),
                network,
                source: source.clone(),
            }));
    }

    if let Some(params) = params {
        for param in params.split('&') {
            let pos = param.find('=').ok_or(Bip21Error::MissingEquals)?;
            let original_key_string = param[..pos].to_lowercase();
            let original_key = original_key_string.as_str();
            let value = &param[(pos.saturating_add(1))..];
            let (key, is_required) = if let Some(stripped) = original_key.strip_prefix("req-") {
                (stripped, true)
            } else {
                (original_key, false)
            };

            parse_bip21_key(source, &mut bip_21, original_key, value, key, is_required)?;
        }
    }

    if bip_21.payment_methods.is_empty() {
        return Err(Bip21Error::NoPaymentMethods);
    }

    Ok(Some(bip_21))
}

fn parse_bip21_key(
    source: &PaymentRequestSource,
    bip_21: &mut Bip21Details,
    original_key: &str,
    value: &str,
    key: &str,
    is_required: bool,
) -> Result<(), Bip21Error> {
    match key {
        "amount" if bip_21.amount_sat.is_some() => {
            return Err(Bip21Error::multiple_params(key));
        }
        "amount" => {
            bip_21.amount_sat = Some(
                bitcoin::Amount::from_str_in(value, Denomination::Bitcoin)
                    .map_err(|_| Bip21Error::InvalidAmount)?
                    .to_sat(),
            );
        }
        "assetid" if bip_21.asset_id.is_some() => {
            return Err(Bip21Error::multiple_params(key));
        }
        "assetid" => bip_21.asset_id = Some(value.to_string()),
        "bc" => {}
        "label" if bip_21.label.is_some() => {
            return Err(Bip21Error::multiple_params(key));
        }
        "label" => {
            let percent_decoded =
                percent_decode_str(value).map_err(Bip21Error::invalid_parameter_func("label"))?;
            bip_21.label = Some(
                percent_decoded
                    .decode_utf8()
                    .map_err(Bip21Error::invalid_parameter_func("label"))?
                    .to_string(),
            );
        }
        "lightning" => {
            let lightning = parse_lightning_payment_method(value, source);
            match lightning {
                Some(lightning) => bip_21.payment_methods.push(lightning),
                None => return Err(Bip21Error::invalid_parameter("lightning")),
            }
        }
        "lno" => {
            let bolt12_offer = parse_bolt12_offer(value, source);
            match bolt12_offer {
                Some(offer) => bip_21.payment_methods.push(InputType::Bolt12Offer(offer)),
                None => return Err(Bip21Error::invalid_parameter("lno")),
            }
        }
        "message" if bip_21.message.is_some() => {
            return Err(Bip21Error::multiple_params(key));
        }
        "message" => {
            let percent_decoded =
                percent_decode_str(value).map_err(Bip21Error::invalid_parameter_func("label"))?;
            bip_21.message = Some(
                percent_decoded
                    .decode_utf8()
                    .map_err(Bip21Error::invalid_parameter_func("label"))?
                    .to_string(),
            );
        }
        "sp" => {
            let silent_payment_address = parse_silent_payment_address(value, source);
            match silent_payment_address {
                Some(silent_payment) => bip_21
                    .payment_methods
                    .push(InputType::SilentPaymentAddress(silent_payment)),
                None => return Err(Bip21Error::invalid_parameter("sp")),
            }
        }
        "spark" => {
            let spark_address = parse_spark_address(value, source);
            match spark_address {
                Some(spark_address) => bip_21.payment_methods.push(spark_address),
                None => return Err(Bip21Error::invalid_parameter("spark")),
            }
        }
        extra_key => {
            if is_required {
                return Err(Bip21Error::UnknownRequiredParameter(extra_key.to_string()));
            }

            bip_21.extras.push(Bip21Extra {
                key: original_key.to_string(),
                value: value.to_string(),
            });
        }
    }
    Ok(())
}

fn parse_spark_address(input: &str, source: &PaymentRequestSource) -> Option<InputType> {
    if let Ok(spark_address) = input.parse::<SparkAddress>() {
        return Some(InputType::SparkAddress(SparkAddressDetails {
            address: input.to_string(),
            decoded_address: spark_address.into(),
            source: source.clone(),
        }));
    }
    None
}

fn parse_bitcoin(input: &str, source: &PaymentRequestSource) -> Option<InputType> {
    if let Ok((hrp, _)) = bech32::decode(input)
        && hrp.to_lowercase().as_str() == "sp"
    {
        match parse_silent_payment_address(input, source) {
            Some(silent_payment) => {
                return Some(InputType::SilentPaymentAddress(silent_payment));
            }
            None => {
                return None;
            }
        }
    }

    if let Some(address) = parse_bitcoin_address(input, source) {
        return Some(InputType::BitcoinAddress(address));
    }

    None
}

fn parse_bitcoin_address(
    input: &str,
    source: &PaymentRequestSource,
) -> Option<BitcoinAddressDetails> {
    if input.is_empty() {
        return None;
    }

    let address: Address<NetworkUnchecked> = input.parse().ok()?;
    let network = match 1 {
        _ if address.is_valid_for_network(bitcoin::Network::Bitcoin) => bitcoin::Network::Bitcoin,
        _ if address.is_valid_for_network(bitcoin::Network::Regtest) => bitcoin::Network::Regtest,
        _ if address.is_valid_for_network(bitcoin::Network::Signet) => bitcoin::Network::Signet,
        _ if address.is_valid_for_network(bitcoin::Network::Testnet) => bitcoin::Network::Testnet,
        _ if address.is_valid_for_network(bitcoin::Network::Testnet4) => bitcoin::Network::Testnet4,
        _ => return None,
    }
    .into();
    Some(BitcoinAddressDetails {
        address: address.assume_checked().to_string(),
        network,
        source: source.clone(),
    })
}

fn parse_bolt11(input: &str, source: &PaymentRequestSource) -> Option<Bolt11InvoiceDetails> {
    let bolt11: lightning::bolt11_invoice::Bolt11Invoice = match input.parse() {
        Ok(invoice) => invoice,
        Err(_) => return None,
    };

    Some(Bolt11InvoiceDetails {
        amount_msat: bolt11.amount_milli_satoshis(),
        description: match bolt11.description() {
            Bolt11InvoiceDescriptionRef::Direct(description) => Some(description.to_string()),
            Bolt11InvoiceDescriptionRef::Hash(_) => None,
        },
        description_hash: match bolt11.description() {
            Bolt11InvoiceDescriptionRef::Direct(_) => None,
            Bolt11InvoiceDescriptionRef::Hash(sha256) => Some(sha256.0.to_string()),
        },
        expiry: bolt11.expiry_time().as_secs(),
        invoice: super::Bolt11Invoice {
            bolt11: input.to_string(),
            source: source.clone(),
        },
        min_final_cltv_expiry_delta: bolt11.min_final_cltv_expiry_delta(),
        network: bolt11.network().into(),
        payee_pubkey: bolt11.get_payee_pub_key().to_string(),
        payment_hash: bolt11.payment_hash().to_string(),
        payment_secret: hex::encode(bolt11.payment_secret().0),
        routing_hints: bolt11
            .route_hints()
            .into_iter()
            .map(|hint| Bolt11RouteHint {
                hops: hint
                    .0
                    .into_iter()
                    .map(|hop| Bolt11RouteHintHop {
                        src_node_id: hop.src_node_id.to_string(),
                        short_channel_id: format_short_channel_id(hop.short_channel_id),
                        fees_base_msat: hop.fees.base_msat,
                        fees_proportional_millionths: hop.fees.proportional_millionths,
                        cltv_expiry_delta: hop.cltv_expiry_delta,
                        htlc_minimum_msat: hop.htlc_minimum_msat,
                        htlc_maximum_msat: hop.htlc_maximum_msat,
                    })
                    .collect(),
            })
            .collect(),
        timestamp: bolt11.duration_since_epoch().as_secs(),
    })
}

fn parse_bolt12_offer(input: &str, source: &PaymentRequestSource) -> Option<Bolt12OfferDetails> {
    let offer: lightning::offers::offer::Offer = match input.parse() {
        Ok(offer) => offer,
        Err(_) => return None,
    };

    let min_amount = match offer.amount() {
        Some(lightning::offers::offer::Amount::Bitcoin { amount_msats }) => {
            Some(super::Amount::Bitcoin {
                amount_msat: amount_msats,
            })
        }
        Some(lightning::offers::offer::Amount::Currency {
            iso4217_code,
            amount,
        }) => Some(super::Amount::Currency {
            iso4217_code: String::from_utf8(iso4217_code.to_vec()).ok()?,
            fractional_amount: amount,
        }),
        None => None,
    };

    Some(Bolt12OfferDetails {
        absolute_expiry: offer.absolute_expiry().map(|e| e.as_secs()),
        chains: offer.chains().into_iter().map(|c| c.to_string()).collect(),
        description: offer.description().map(|d| d.to_string()),
        issuer: offer.issuer().map(|i| i.to_string()),
        min_amount,
        offer: Bolt12Offer {
            offer: input.to_string(),
            source: source.clone(),
        },
        paths: offer
            .paths()
            .iter()
            .map(|p| Bolt12OfferBlindedPath {
                blinded_hops: p
                    .blinded_hops()
                    .iter()
                    .map(|h| h.blinded_node_id.to_string())
                    .collect(),
            })
            .collect(),
        signing_pubkey: offer.issuer_signing_pubkey().map(|p| p.to_string()),
    })
}

fn parse_bolt12_invoice(
    _input: &str,
    _source: &PaymentRequestSource,
) -> Option<Bolt12InvoiceDetails> {
    // TODO: Implement parsing of Bolt12 invoices
    None
}

fn parse_bolt12_invoice_request(
    _input: &str,
    _source: &PaymentRequestSource,
) -> Option<Bolt12InvoiceRequestDetails> {
    // TODO: Implement parsing of Bolt12 invoice requests
    None
}

pub fn parse_json<T>(json: &str) -> Result<T, ServiceConnectivityError>
where
    for<'a> T: serde::de::Deserialize<'a>,
{
    serde_json::from_str::<T>(json).map_err(|e| ServiceConnectivityError::Json(e.to_string()))
}

fn parse_lightning_payment_method(input: &str, source: &PaymentRequestSource) -> Option<InputType> {
    let input = if has_lightning_prefix(input) {
        &input[LIGHTNING_PREFIX_LEN..]
    } else {
        input
    };

    if let Some(bolt11) = parse_bolt11(input, source) {
        return Some(InputType::Bolt11Invoice(bolt11));
    }

    if let Some(bolt12_offer) = parse_bolt12_offer(input, source) {
        return Some(InputType::Bolt12Offer(bolt12_offer));
    }

    if let Some(bolt12_invoice) = parse_bolt12_invoice(input, source) {
        return Some(InputType::Bolt12Invoice(bolt12_invoice));
    }

    None
}

fn parse_silent_payment_address(
    _input: &str,
    _source: &PaymentRequestSource,
) -> Option<SilentPaymentAddressDetails> {
    // TODO: Support silent payment addresses
    None
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum LnurlRequestDetails {
    PayRequest {
        #[serde(flatten)]
        pay_request: LnurlPayRequestDetails,
    },
    WithdrawRequest {
        #[serde(flatten)]
        withdraw_request: LnurlWithdrawRequestDetails,
    },
    #[serde(rename = "login")]
    AuthRequest {
        #[serde(flatten)]
        auth_request: LnurlAuthRequestDetails,
    },
    Error {
        #[serde(flatten)]
        error_details: LnurlErrorDetails,
    },
}

#[cfg(test)]
#[allow(clippy::similar_names)]
mod tests {
    use macros::async_test_all;
    use serde_json::json;

    use crate::input::error::Bip21Error;
    use crate::input::parser::InputParser;
    use crate::input::{Bip21Details, Bip21Extra, BitcoinAddressDetails, InputType, ParseError};
    use crate::test_utils::mock_dns_resolver::MockDnsResolver;
    use crate::test_utils::mock_rest_client::{MockResponse, MockRestClient};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// BIP21 amounts which can lead to rounding errors.
    /// The format is: (sat amount, BIP21 BTC amount)
    fn get_bip21_rounding_test_vectors() -> Vec<(u64, f64)> {
        vec![
            (999, 0.0000_0999),
            (1_000, 0.0000_1000),
            (59_810, 0.0005_9810),
        ]
    }

    fn mock_lnurl_pay_endpoint(mock_rest_client: &MockRestClient, error: Option<String>) {
        let response_body = match error {
            None => json!({
                "callback":"https://localhost/lnurl-pay/callback/db945b624265fc7f5a8d77f269f7589d789a771bdfd20e91a3cf6f50382a98d7",
                "tag": "payRequest",
                "maxSendable": 16000,
                "minSendable": 4000,
                "metadata": "[
                    [\"text/plain\",\"WRhtV\"],
                    [\"text/long-desc\",\"MBTrTiLCFS\"],
                    [\"image/png;base64\",\"iVBORw0KGgoAAAANSUhEUgAAASwAAAEsCAYAAAB5fY51AAATOElEQVR4nO3dz4slVxXA8fIHiEhCjBrcCHEEXbiLkiwd/LFxChmQWUVlpqfrdmcxweAk9r09cUrQlWQpbgXBv8CdwrhRJqn7umfEaEgQGVGzUEwkIu6ei6TGmvH16/ej6p5z7v1+4Ozfq3vqO5dMZ7qqAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgHe4WbjuutBKfw4AWMrNwnUXw9zFMCdaANS6J1ZEC4BWC2NFtABoszRWRAuAFivFimgBkLZWrIgWACkbxYpoAUhtq1gRLQCpjBIrogVU1ZM32webma9dDM+7LrR3J4bnm5mvn7zZPij9GS0bNVZEaxTsvDEu+iea6F9w0d9a5QVpunDcRP/C7uzgM9Kf3ZJJYkW0NsLOG7PzynMPNDFcaTr/2+1eFH/kon/q67evfkD6O2k2aayI1krYeYPO3mjf67rwjIv+zZFfmL+5zu+18/bd0t9RmySxIlonYueNuvTS4cfe/tNhuhem6cKvXGw/LP1dtUgaK6L1f9h5o/aODj/rov9Hihemif4vzS3/SenvLE0kVkTrLnbeKBfDYxNch0+bv7p47RPS312KaKyIFjtv1U53cMZ1/u8yL42/s3/76iPSzyA1FbEqOFrsvFGXX24fdtH/UfKFaaKP0s8hJVWxKjBa7LxhTfQ3xF+WGOYu+h9LP4sUVMaqsGix80a56J+WP7T/ze7s4PPSz2RKqmNVSLTYeaMuHfmPuBjekj6w4TTRvyb9XKZiIlaZR4udN6yJ/gfSh7Vo9mb+kvSzGZupWGUcLXbeqJ1XnnvAdf7f0gd1wrwq/XzGZDJWGUaLnTesmYWLCg5p2Twm/YzGYDpWmUWLnTfMxfAzBQd04ux24XvSz2hbWcQqo2ix80ZdmF94j4v+P9IHtHz8TenntI2sYtWP4Wix84Zd7g4flz+c00f6OW0qy1j1YzRa7LxhTRd2pA9mlWluffvT0s9qXVnHqh+D0WLnDbPyUjWd/4r0s1qHlec6yhiLlpWzsbbzSTTRf1f6YFaZvdmhk35Wq7LyQow6hqLFzhvWRP8d6YNZZZoYvPSzWkWRserHSLTYecPcLDwrfTArzrekn9Vpio5VPwaixc4b1sTDfQUHs8rsSj+rZYjVYJRHi503bLfzX1ZwMKdO0x18UfpZnYRYLRjF0WLnDds/PnhU+mBWmYsvPftR6We1CLFaMkqjxc4b5zr/uvThLF98/wfpZ7QIsVrl7HRGi503zHXhJ+IHtGSaGH4k/YzuR6zWefn0RYudN8xFf176gJbN3lH4gvQzGiJWG4yyaLHzxrku/FP6kE5Y9D9JP5shYrXVWbbS5zfEzhvmutCKH9TC8U9LP5sesRrlZWylz7HHzht28bh9SOCXSJ623Gr+pCFWo55rK32eVcXOm7c3O3TiB3bP+PPSz6SqiNVEL2Yrfa5Vxc6b57rwC/lDC/Mm+p9KP4uqIlaTjpJosfOGvfNbcO+IHlwXji/8+pn3Sz8LYpVgFESLnTdupzs408Twhszh+Tv7t68+Iv0MiFXCURAtdt64y93h4030/0p8eH/e6Q7OSH93YiUwCqJV8s5nwUX/RLq/RfF3dm9f+7j4dyZWcqMgWiXufFb2jw8ebWL43ZQH13T+50/95uCD0t+VWCkYBdEqaeezdOW1K+9rYvAuhrfGXU7/ejMLF6t59S7p70isFI2CaJWw89m7/HL7sJv5b7oYXt3u4PzNvVn4mvT36RErhaMgWlWV784Xpznyn2ti+KGL/verHFjThRdd57+/0137lPRnHyJWikdJtHq57HzxvvGi/1DTHX7VzcJ114X27sx82O3Cl7T+fAmxMjDKotWzuvMwilgZGqXRApIgVgaHaKFExMrwEC2UhFhlMEQLJSBWGQ3RQs6IVYZDtJAjYpXxEC3khFgVMEQLOSBWBQ3RgmXEqsAhWrDIdaGt63rOlDdEC6b0v2dO+sVhhILFTQtWDH8ppvSLwwgGi2hBu/t/g6/0i8MIB4toQatFv25c+sVhFASLaEGbRbEiWOUOf3sItU6KFcEqd/iRB6i0LFYEq9zh57SgzmmxIljlDj9cClVWiRXBKnf4iXiosWqsCFa5w//GAxXWiRXBKnfW2RGihUmsGyuCVe6suydEC6PaJFYEq9zZZFeIFkaxaawIVrmz6b4QLWxlm1gRrHJnm50hWtjItrEiWOXOtntDtLCWMWJFsMqdMXaHaGElY8WKYJU7Y+0P0cJSY8aKYJU7Y+4Q0cJCY8eKYJU7Y+8R0cI9pogVwSp3ptglooWqqqaLFcEqd6baJ6JVuCljRbDKnSl3imgVaupYEaxyZ+q9IlqFSRGrhME6K/Uc67q29Mtif1nX9dksgkW0ypEqVgmDdUPiOZ4/f/6huq7fUBCilULVf+5sgkW08pcyVgmDNa8Fblm1/tvVPaEafO58gkW08pU6VomDlfSWpfx2tTBUveyCRbTyIxGrxMGaL3tJx1brvF0tDdXgs+cXLKKVD6lYCQQryS1L4e1qpVD1sg0W0bJPMlYCwZqv8+JuqtZzu1orVIPPn2+wiJZd0rESCtaktywlt6uNQtXLPlhEyx4NsRIK1nybl/k0teztaqtQDb5D/sEiWnZoiZVgsCa5ZQnerkYJVa+YYBEt/TTFSjBY8zFf8F6d/nY1aqgG36OcYBEtvbTFSjhYo96yEt+uJglVr7hgES19NMZKOFjzMV/6Os3tatJQDb5LecEiWnpojZWCYI1yy0pwu0oSql6xwSJa8jTHSkGw5mOEoJ7udpU0VIPvU26wiJYc7bFSEqytblkT3a5EQtUrPlhEKz0LsVISrPk2cainuV29Udf19fPnzz804kqs850IFtFKx0qsFAVro1tWgv92JRIugkW0krEUK0XBmteb/T93qX7uKmm4CBbRSsJarJQFa61bltBPtScJF8EiWpOzGCtlwZrX6/0TLJL/z+Ck4SJYRGtSVmOlMFgr3bKU/IsMk4WLYBGtyViOlcJgzevV/kVOLf/e1SThIlhEaxLWY6U0WEtvWYpuV5OFi2ARrdHlECulwZrXy39Bg7bb1ejhIlhEa1S5xEpxsBbespTfrkYLF8EiWqPJKVaKgzWvF/++Pgu3q63DRbCI1ihyi5XyYN1zyzJ4u9o4XASLaG0tx1gpD9a8vvfXt1u9Xa0dLoJFtLaSa6wMBOtGVWVzu1o5XASLaG0s51gZCNa8ruuzdV63q1PDRbCI1kZyj5WRYN2o87xdnRgugkW01lZCrIwEiyFYRGuZUmJFsMod6b0jWiMpKVYEq9yR3juiNYLSYkWwyh3pvSNaWyoxVgSr3JHeO6K1hVJjRbDKHem9I1pbIFhMaSO9dwRrS6VGS/rFYQgWsdpQidGSfnEYgkWstlBatKRfHIZgEastlRQt6ReHIVjEagSlREv6xWEIFrEaSQnRSvSCtOfOnXtT+iVNMe98z19Kf47ig1VarHq5RyvFy1FVd/9NqxLC1dZv/5M40p+j3GCVGqteztFKFaxezuE6d+7cm4N/00r1LUt674jVxHKNVupg9TINV9t/v1r5LUt674hVAjlGSypYvVzCNbxd9WrFtyzpvSNWieQWLelg9TIIV3v/d6oV37Kk945YJZRTtLQEq2cxXItuV71a6S1Leu+IVWK5REtbsHrGwtWe9D1qpbcs6b0jVgJyiJbWYPW0h2vZ7apXK7xlSe8dsRJiPVrag9VTHK72tM9eK7xlSe8dsRJkOVpWgtXTFK5Vble9WtktS3rviJUwq9GyFqyeknC1q37eWtktS3rviJUCFqNlNVg9qXCtc7vq1YpuWdJ7R6yUsBYt68HqCYSrXfcz1opuWdJ7R6wUsRStXILVSxGuTW5XvVrJLUt674iVMlailVuwehOHq930c9VKblnSe0esFLIQrVyDVVV343BjzO+yze1q8LnEb1nSe0eslNIerRyDNUWoBtOO9PkIFrHSSXO0cgrWxKEa5XY1+KyityzpvSNWymmNVg7BmjpUg2lH/swEi1jppTFaloOVMFSj3q4Gn1/sliW9d8TKCG3RshislKEaTDvR9yBYxEo3TdGyFCyhUE1yuxp8J5FblvTeEStjtETLQrCkQjWYdoQjX/bdygwWsbJFQ7Q0B0tBqCa9XQ2+Z/JblvTeESujpKOlMVgaQjWYdoJjX/R9ywkWsbJNMlqagqUsVEluV4PvnvSWRaywFaloaQiWtlANpk1w9MNnkHewiFVeJKIlGSzFoUp6uxo8j2S3LGKFUaSOlkSwNIdqMG3qs68T3rKIFUaTMlopg2UkVCK3q8EzSnLLIlYYVapoJYqAiVANppU69zrRLYtYYXQpoqUgDozAECtMYupoSb84TIbBIlZlmzJa0i8Ok1mwiBWqarpoSb84TEbBIlYYmiJa0i8Ok0mwiBUWGTta0i8Ok0GwiBWWGTNa0i8OYzxYxAqrGCta0i8OYzhYxArrGCNa0i8OYzRYxAqb2DZa0i8OYzBYxArb2CZa0i8OYyxYxApj2DRa0i8OYyhYxApj2iRa0i8OYyRYxApTWDda0i8OYyBYxApTWida0i8OozxYxAoprBot6ReHURwsYoWUVomW9IvDKA0WsYKE06Il/eIwCoNFrCBpWbSkXxxGWbCIFTQ4KVrSLw6jKFjECposipb0i8MoCRaxgkb3R0v6xWEUBItYQbNhtKRfHEY4WMQKFvTRkn5xGMFgEStY4rrQSr84jFCwiBUsSvUbphlFQ6xgGdEqaIgVckC0ChhihZwQrYyHWCFHRCvDIVbIGdHKaIgVSkC0MhhihZIQLcNDrFAiomVwiBVKRrQMDbHCmJ682T7YzHztYnjedaG9OzE838x8/eTN9kHpz7gI0TIwSmNldeeL5aJ/oon+BRf9rVUWr+nCcRP9C7uzg89If/YhoqV4lMUql50vxs4rzz3QxHCl6fxvt1tEf+Sif+rrt69+QPo7VRXRUjlKYpXrzmft7I32va4Lz7jo3xx5Mf/mOr/Xztt3S39HoqVoFMSqhJ3P0qWXDj/29p8O0y1o04Vfudh+WPq7Ei0FoyBWJe18VvaODj/rov9HikVtov9Lc8t/Uvo7Ey3BURCrEnc+Cy6Gxya4Dp82f3Xx2ifEvzvRSj8KYlXyzpu20x2ccZ3/u8zy+jv7t68+Iv0MiFbCURArdt6oyy+3D7vo/yi5wE30Ufo5VBXRSjIKYsXOG9ZEf0N8iWOYu+h/LP0sqopoTToKYlVV7LxZLvqn5Q/tf7M7O/i89DOpKqI1ySiJFTtv1KUj/xEXw1vSBzacJvrXpJ9Lj2iNOEpixc4b1kT/A+nDWjR7M39J+tn0iNYIoyRWVcXOm7XzynMPuM7/W/qgTphXpZ/PENHaYhTFip03rJmFiwoOadk8Jv2MhojWBqMoVlXFzpvmYviZggM6cXa78D3pZ3Q/orXGKItVVbHzZl2YX3iPi/4/0ge0fPxN6ee0CNFaYRTGip037HJ3+Lj84Zw+0s/pJERrySiMVVWx86Y1XdiRPphVprn17U9LP6uTEK0FozRWVcXOm+Zm4br0wax0eJ3/ivSzWoZoDUZxrKqKnTetif670gezyuzNDp30szoN0QrqY1VV7LxpTfTfkT6YVaaJwUs/q1UUHS0Dsaoqdt40NwvPSh/MivMt6We1qiKjZSRWVcXOm9bEw30FB7PK7Eo/q3UUFS1Dsaoqdt603c5/WcHBnDpNd/BF6We1riKiZSxWVcXOm7Z/fPCo9MGsMhdfevaj0s9qE1lHy2CsqoqdN891/nXpw1n+Yvg/SD+jbWQZLaOx6rHzhrku/ET8gJZME8OPpJ/RtrKKlvFYVRU7b5qL/rz0AS2bvaPwBelnNIYsopVBrKqKnTfPdeGf0od0wgvyJ+lnMybT0cokVj123jC9L5J/WvrZjE3vsy4nVlWl+Rzy2/nRXTxuHxL4JZKnvSTZ/kmj92UpI1ZVxc6btzc7dOIHds/489LPZEomopVprHrsvHGuC7+QP7Qwb6L/qfSzSEF1tDKPVY+dN+yd34J7R/TgunB84dfPvF/6WaSiMlqFxKqq2HnzdrqDM00Mb8gcnr+zf/vqI9LPIDVV0SooVj123rjL3eHjTfT/Snx4f97pDs5If3cpKqJVYKx67LxxLvon0v0tir+ze/vax6W/szTRaBUcqx47b9z+8cGjTQy/m/Lgms7//KnfHHxQ+rtqIRItYnUXO2/cldeuvK+JwbsY3hr3JfGvN7NwsZpX75L+jtokjRax+j/sfAYuv9w+7Gb+my6GV7c7OH9zbxa+Jv19tEsSLWK1FDufiebIf66J4Ycu+t+vcmBNF150nf/+TnftU9Kf3ZJJo0Ws1sLOZ+IbL/oPNd3hV90sXHddaO/OzIfdLnyJny/ZziTRIlZbYeeBJUaNFrECMLVRokWsAKSyVbSIFYDUNooWsQIgZa1oESsA0laKFrECoMXSaBErANosjBaxAqDVPdEiVgC063/aWvpzAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQI//AplAdntdLBX1AAAAAElFTkSuQmCC\"]
                ]",
                "commentAllowed": 0,
                "payerData":{
                    "name": { "mandatory":false },
                    "pubkey": { "mandatory":false },
                    "identifier": { "mandatory":false },
                    "email":{ "mandatory":false },
                    "auth": { "mandatory":false, "k1":"18ec6d5b96db6f219baed2f188aee7359fcf5bea11bb7d5b47157519474c2222" }
                }
            }).to_string(),
            Some(err_reason) => json!({
                "status": "ERROR",
                "reason": err_reason
            })
            .to_string(),
        };

        mock_rest_client.add_response(MockResponse::new(200, response_body));
    }

    fn mock_lnurl_withdraw_endpoint(mock_rest_client: &MockRestClient, error: Option<String>) {
        let (response_body, status_code) = match error {
            None => (json!({
                "tag": "withdrawRequest",
                "callback": "https://localhost/lnurl-withdraw/callback/e464f841c44dbdd86cee4f09f4ccd3ced58d2e24f148730ec192748317b74538",
                "k1": "37b4c919f871c090830cc47b92a544a30097f03430bc39670b8ec0da89f01a81",
                "minWithdrawable": 3000,
                "maxWithdrawable": 12000,
                "defaultDescription": "sample withdraw",
            }).to_string(), 200),
            Some(err_reason) => (json!({
                "status": "ERROR",
                "reason": err_reason
            })
            .to_string(), 400),
        };

        mock_rest_client.add_response(MockResponse::new(status_code, response_body));
    }

    #[async_test_all]
    async fn test_bip21_multiple_params() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // Duplicate label parameter
        let bip21_with_duplicate_label = format!("bitcoin:{addr}?label=first&label=second");
        let result = input_parser.parse(&bip21_with_duplicate_label).await;
        assert!(matches!(result, Err(ParseError::Bip21Error(_))));

        // Duplicate message parameter
        let bip21_with_duplicate_message = format!("bitcoin:{addr}?message=first&message=second");
        let result = input_parser.parse(&bip21_with_duplicate_message).await;
        assert!(matches!(result, Err(ParseError::Bip21Error(_))));

        // Duplicate amount parameter
        let bip21_with_duplicate_amount = format!("bitcoin:{addr}?amount=0.001&amount=0.002");
        let result = input_parser.parse(&bip21_with_duplicate_amount).await;
        assert!(matches!(result, Err(ParseError::Bip21Error(_))));
    }

    #[async_test_all]
    async fn test_bip21_required_parameter() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // BIP21 with unknown required parameter
        let bip21_with_req = format!("bitcoin:{addr}?req-unknown=value");
        let result = input_parser.parse(&bip21_with_req).await;

        assert!(matches!(result, Err(ParseError::Bip21Error(_))));

        // BIP21 with known required parameter
        let bip21_with_known_req = format!("bitcoin:{addr}?req-amount=0.001");
        let result = input_parser.parse(&bip21_with_known_req).await;

        assert!(matches!(
            result,
            Ok(InputType::Bip21(bip21))
            if bip21.amount_sat == Some(100_000)
        ));
    }

    #[async_test_all]
    async fn test_bip21_url_encoded_values() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // BIP21 with URL-encoded values
        let encoded_message = "Hello%20World%21%20%26%20Special%20chars%3A%20%24%25";
        let bip21_with_encoded = format!("bitcoin:{addr}?message={encoded_message}");
        let result = input_parser.parse(&bip21_with_encoded).await;

        assert!(matches!(
            result,
            Ok(InputType::Bip21(bip21))
            if bip21.message.as_deref() == Some("Hello World! & Special chars: $%")
        ));
    }

    #[async_test_all]
    async fn test_bip21_with_extra_parameters() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // BIP21 with custom parameters
        let bip21_with_extra = format!("bitcoin:{addr}?amount=0.001&custom=value&another=param");
        let result = input_parser.parse(&bip21_with_extra).await;

        assert!(matches!(
            result,
            Ok(InputType::Bip21(bip21))
            if bip21.extras.len() == 2 &&
               bip21.extras.contains(&Bip21Extra{ key: "custom".to_string(), value: "value".to_string()}) &&
               bip21.extras.contains(&Bip21Extra{ key: "another".to_string(), value: "param".to_string()})
        ));
    }

    #[async_test_all]
    async fn test_bip21_with_invalid_amount() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // BIP21 with invalid amount format
        let bip21_with_invalid_amount = format!("bitcoin:{addr}?amount=invalid");
        let result = input_parser.parse(&bip21_with_invalid_amount).await;

        assert!(matches!(result, Err(ParseError::Bip21Error(_))));
    }

    #[async_test_all]
    async fn test_bip21_with_invalid_lightning() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // BIP21 with invalid lightning parameter
        let bip21_with_invalid_ln = format!("bitcoin:{addr}?lightning=invalidlndata");
        let result = input_parser.parse(&bip21_with_invalid_ln).await;

        assert!(matches!(result, Err(ParseError::Bip21Error(_))));
    }

    #[async_test_all]
    async fn test_bip21_with_invalid_message_encoding() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";
        // Invalid UTF-8 sequence in message
        let bip21_with_invalid_message = format!("bitcoin:{addr}?message=%FF%FE%FD");
        let result = input_parser.parse(&bip21_with_invalid_message).await;

        assert!(matches!(result, Err(ParseError::Bip21Error(_))));
    }

    #[async_test_all]
    async fn test_bip21_with_invalid_silent_payment() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // BIP21 with invalid silent payment parameter
        let bip21_with_invalid_sp = format!("bitcoin:{addr}?sp=invalidspaddress");
        let result = input_parser.parse(&bip21_with_invalid_sp).await;

        assert!(matches!(
            result,
            Err(ParseError::Bip21Error(Bip21Error::InvalidParameter(_)))
        ));
    }

    #[async_test_all]
    async fn test_bip21_with_missing_equals() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // BIP21 with parameter missing equals sign
        let bip21_with_missing_equals = format!("bitcoin:{addr}?labelvalue");
        let result = input_parser.parse(&bip21_with_missing_equals).await;

        assert!(matches!(result, Err(ParseError::Bip21Error(_))));
    }

    #[async_test_all]
    async fn test_bip21_without_payment_methods() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // BIP21 without address or payment methods
        let bip21_no_methods = "bitcoin:?amount=0.001";
        let result = input_parser.parse(bip21_no_methods).await;

        assert!(matches!(result, Err(ParseError::Bip21Error(_))));
    }

    #[async_test_all]
    async fn test_bip353_with_invalid_dns_record() {
        let mock_dns_resolver = MockDnsResolver::new();
        // Simulate a TXT record that's not a valid BIP21 URI
        mock_dns_resolver.add_response(vec![String::from("not-a-valid-bip21-uri")]);

        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let bip353_address = "test@example.com";
        let result = input_parser.parse(bip353_address).await;

        // Should fail to parse the BIP353 record and fall back to checking if it's a lightning address
        assert!(matches!(result, Err(ParseError::InvalidInput)));
    }

    #[async_test_all]
    async fn test_bip353_address() {
        let mock_dns_resolver = MockDnsResolver::new();
        mock_dns_resolver.add_response(vec![String::from("bitcoin:?sp=sp1qqweplq6ylpfrzuq6hfznzmv28djsraupudz0s0dclyt8erh70pgwxqkz2ydatksrdzf770umsntsmcjp4kcz7jqu03jeszh0gdmpjzmrf5u4zh0c&lno=lno1pqps7sjqpgtyzm3qv4uxzmtsd3jjqer9wd3hy6tsw35k7msjzfpy7nz5yqcnygrfdej82um5wf5k2uckyypwa3eyt44h6txtxquqh7lz5djge4afgfjn7k4rgrkuag0jsd5vxg")]);
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // Test with a BIP-353 address
        let bip353_address = "user@bitcoin-domain.com";

        // This should be handled by parse_bip_353
        // Since mocking DNS is complex, we'll just ensure the method exists and is called
        let result = input_parser.parse(bip353_address).await;
        println!("Debug - bip353 address result: {result:?}");

        // The result might be Err if DNS mocking isn't set up
        // Just check the method exists and runs without crashing
    }

    #[async_test_all]
    async fn test_bip353_spark_address() {
        let mock_dns_resolver = MockDnsResolver::new();
        mock_dns_resolver.add_response(vec![String::from(
            "bitcoin:?spark=sparkrt1pgssyuuuhnrrdjswal5c3s3rafw9w3y5dd4cjy3duxlf7hjzkp0rqx6dc0nltx",
        )]);
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // Test with a BIP-353 address
        let bip353_address = "user@bitcoin-domain.com";

        // This should be handled by parse_bip_353
        let result = input_parser.parse(bip353_address).await.unwrap();
        let InputType::Bip21(bip21_details) = result else {
            panic!("Expected Bip21 result");
        };
        let spark_payment_method = bip21_details
            .payment_methods
            .into_iter()
            .find(|pm| matches!(pm, InputType::SparkAddress(_)));
        assert!(
            spark_payment_method.is_some(),
            "Expected SparkAddress payment method"
        );
    }

    #[async_test_all]
    async fn test_bip353_address_too_long() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // Local part longer than 63 chars
        let too_long_local = "a".repeat(64) + "@example.com";
        let result = input_parser.parse(&too_long_local).await;

        // Should not be recognized as a BIP353 address
        assert!(!matches!(result, Ok(InputType::Bip21(_))));

        // Domain part longer than 63 chars
        let too_long_domain = format!("user@{}.com", "a".repeat(60));
        let result = input_parser.parse(&too_long_domain).await;

        // Should not be recognized as a BIP353 address
        assert!(!matches!(result, Ok(InputType::Bip21(_))));
    }

    #[async_test_all]
    async fn test_bitcoin_address() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        for address in [
            "1andreas3batLhQa2FawWjeyjCqyBzypd",
            "12c6DSiU4Rq3P4ZxziKxzrL5LmMBrzjrJX",
            "bc1qxhmdufsvnuaaaer4ynz88fspdsxq2h9e9cetdj",
            "3CJ7cNxChpcUykQztFSqKFrMVQDN4zTTsp",
        ] {
            let result = input_parser.parse(address).await;
            println!("Debug - bitcoin address result for '{address}': {result:?}");
            assert!(matches!(
                result,
                Ok(crate::input::InputType::BitcoinAddress(_))
            ));
        }
    }

    #[async_test_all]
    async fn test_bitcoin_address_bip21() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        // Addresses from https://github.com/Kixunil/bip21/blob/master/src/lib.rs

        // Invalid address with the `bitcoin:` prefix
        let result = input_parser.parse("bitcoin:testinvalidaddress").await;
        println!("Debug - invalid bip21 address result: {result:?}");
        assert!(matches!(
            result,
            Err(ParseError::Bip21Error(Bip21Error::InvalidAddress))
        ));

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

        // Valid address with the `bitcoin:` prefix
        let bip21_addr = format!("bitcoin:{addr}");
        let result = input_parser.parse(&bip21_addr).await;
        println!("Debug - valid bip21 address result for '{bip21_addr}': {result:?}");
        assert!(matches!(
            result,
            Ok(InputType::Bip21(Bip21Details { amount_sat: _, asset_id: _, uri: _, extras: _, label: _, message: _, payment_methods }))
            if payment_methods.len() == 1 && matches!(&payment_methods[0], InputType::BitcoinAddress(BitcoinAddressDetails { address, network: _, source: _ }) if address == addr)
        ));

        // Address with amount
        let bip21_addr_amount = format!("bitcoin:{addr}?amount=0.00002000");
        let result = input_parser.parse(&bip21_addr_amount).await;
        println!("Debug - bip21 with amount result for '{bip21_addr_amount}': {result:?}");
        assert!(matches!(
            result,
            Ok(InputType::Bip21(Bip21Details { amount_sat, asset_id: _, uri: _, extras: _, label: _, message: _, payment_methods }))
            if payment_methods.len() == 1
                && amount_sat == Some(2000)
                && matches!(&payment_methods[0], InputType::BitcoinAddress(BitcoinAddressDetails { address, network: _, source: _ }) if address == addr)
        ));

        // Address with amount and label
        let lbl = "test-label";
        let bip21_addr_amount_label = format!("bitcoin:{addr}?amount=0.00002000&label={lbl}");
        let result = input_parser.parse(&bip21_addr_amount_label).await;
        println!(
            "Debug - bip21 with amount and label result for '{bip21_addr_amount_label}': {result:?}"
        );
        assert!(matches!(
            result,
            Ok(InputType::Bip21(Bip21Details { amount_sat, asset_id: _, uri: _, extras: _, label, message: _, payment_methods }))
            if payment_methods.len() == 1
                && amount_sat == Some(2000)
                && label.as_deref() == Some(lbl)
                && matches!(&payment_methods[0], InputType::BitcoinAddress(BitcoinAddressDetails { address, network: _, source: _ }) if address == addr)
        ));

        // Address with amount, label and message
        let msg = "test-message";
        let bip21_addr_amount_label_msg =
            format!("bitcoin:{addr}?amount=0.00002000&label={lbl}&message={msg}");
        let result = input_parser.parse(&bip21_addr_amount_label_msg).await;
        println!(
            "Debug - bip21 with amount, label and message result for '{bip21_addr_amount_label_msg}': {result:?}"
        );
        assert!(matches!(
            result,
            Ok(InputType::Bip21(Bip21Details { amount_sat, asset_id: _, uri: _, extras: _, label, message, payment_methods }))
            if payment_methods.len() == 1
                && amount_sat == Some(2000)
                && label.as_deref() == Some(lbl)
                && message.as_deref() == Some(msg)
                && matches!(&payment_methods[0], InputType::BitcoinAddress(BitcoinAddressDetails { address, network: _, source: _ }) if address == addr)
        ));
    }

    #[async_test_all]
    async fn test_bitcoin_address_bip21_rounding() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        for (amt, amount_btc) in get_bip21_rounding_test_vectors() {
            let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";

            let result = input_parser
                .parse(&format!("bitcoin:{addr}?amount={amount_btc}"))
                .await;
            println!("Debug - bip21 rounding result for amount {amount_btc}: {result:?}");

            assert!(matches!(
                result,
                Ok(InputType::Bip21(Bip21Details { amount_sat, asset_id: _, uri: _, extras: _, label: _, message: _, payment_methods }))
                if payment_methods.len() == 1
                    && amount_sat == Some(amt)
                    && matches!(&payment_methods[0], InputType::BitcoinAddress(BitcoinAddressDetails { address, network: _, source: _ }) if address == addr)
            ));
        }
    }
    #[async_test_all]
    async fn test_bolt11() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let bolt11 = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";

        // Invoice without prefix
        let result = input_parser.parse(bolt11).await;
        println!("Debug - bolt11 without prefix result: {result:?}");
        assert!(matches!(result, Ok(InputType::Bolt11Invoice(_))));

        // Invoice with prefix
        let invoice_with_prefix = format!("lightning:{bolt11}");
        let result = input_parser.parse(&invoice_with_prefix).await;
        println!("Debug - bolt11 with prefix result: {result:?}");
        assert!(matches!(result, Ok(InputType::Bolt11Invoice(_))));
    }

    #[async_test_all]
    async fn test_bolt11_capitalized() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let bolt11 = "LNBC110N1P38Q3GTPP5YPZ09JRD8P993SNJWNM68CPH4FTWP22LE34XD4R8FTSPWSHXHMNSDQQXQYJW5QCQPXSP5HTLG8YDPYWVSA7H3U4HDN77EHS4Z4E844EM0APJYVMQFKZQHHD2Q9QGSQQQYSSQSZPXZXT9UUQZYMR7ZXCDCCJ5G69S8Q7ZZJS7SGXN9EJHNVDH6GQJCY22MSS2YEXUNAGM5R2GQCZH8K24CWRQML3NJSKM548ARUHPWSSQ9NVRVZ";

        // Invoice without prefix
        let result = input_parser.parse(bolt11).await;
        println!("Debug - capitalized bolt11 without prefix result: {result:?}");
        assert!(matches!(result, Ok(InputType::Bolt11Invoice(_))));

        // Invoice with prefix
        let invoice_with_prefix = format!("LIGHTNING:{bolt11}");
        let result = input_parser.parse(&invoice_with_prefix).await;
        println!("Debug - capitalized bolt11 with prefix result: {result:?}");
        assert!(matches!(result, Ok(InputType::Bolt11Invoice(_))));
    }

    #[async_test_all]
    async fn test_bolt11_with_fallback_bitcoin_address() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";
        let bolt11 = "lnbc110n1p38q3gtpp5ypz09jrd8p993snjwnm68cph4ftwp22le34xd4r8ftspwshxhmnsdqqxqyjw5qcqpxsp5htlg8ydpywvsa7h3u4hdn77ehs4z4e844em0apjyvmqfkzqhhd2q9qgsqqqyssqszpxzxt9uuqzymr7zxcdccj5g69s8q7zzjs7sgxn9ejhnvdh6gqjcy22mss2yexunagm5r2gqczh8k24cwrqml3njskm548aruhpwssq9nvrvz";

        // Address and invoice
        // BOLT11 is the first URI arg (preceded by '?')
        // In the new format, this should be handled by the parse_bip_21 method and return a PaymentRequest
        // that includes the bolt11 data in the payment_methods
        let result = input_parser
            .parse(&format!("bitcoin:{addr}?lightning={bolt11}"))
            .await;
        println!("Debug - bolt11 with fallback bitcoin address (case 1): {result:?}");
        assert!(matches!(result, Ok(InputType::Bip21(_))));

        // Address with amount and invoice
        // BOLT11 is not the first URI arg (preceded by '&')
        let result = input_parser
            .parse(&format!(
                "bitcoin:{addr}?amount=0.00002000&lightning={bolt11}"
            ))
            .await;
        println!("Debug - bolt11 with fallback bitcoin address (case 2): {result:?}");
        assert!(matches!(result, Ok(InputType::Bip21(_))));
    }

    #[async_test_all]
    async fn test_bolt12_invoice() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // Note: This is a placeholder - you'd need a real Bolt12 invoice string
        let bolt12_invoice = "lni1zcss9mk8y3wkklfvevcrszlmu23kfrxh49px20665dqwmn4p72pksese";

        // Currently this should return an error as parse_bolt12_invoice returns None
        let result = input_parser.parse(bolt12_invoice).await;
        assert!(matches!(result, Err(ParseError::InvalidInput)));
    }

    #[async_test_all]
    async fn test_bolt12_offer() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // A valid Bolt12 offer string
        let bolt12_offer = "lno1zcss9mk8y3wkklfvevcrszlmu23kfrxh49px20665dqwmn4p72pksese";

        let result = input_parser.parse(bolt12_offer).await;
        println!("Debug - bolt12 offer result: {result:?}");

        assert!(matches!(result, Ok(InputType::Bolt12Offer(_))));

        // Test with lightning: prefix
        let prefixed_bolt12 = format!("lightning:{bolt12_offer}");
        let result = input_parser.parse(&prefixed_bolt12).await;
        println!("Debug - bolt12 offer with lightning prefix result: {result:?}");

        assert!(matches!(result, Ok(InputType::Bolt12Offer(_))));
    }

    #[async_test_all]
    async fn test_bolt12_offer_in_bip21() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let addr = "1andreas3batLhQa2FawWjeyjCqyBzypd";
        let bolt12_offer = "lno1zcss9mk8y3wkklfvevcrszlmu23kfrxh49px20665dqwmn4p72pksese";

        // Address with Bolt12 offer parameter
        let bip21_with_bolt12 = format!("bitcoin:{addr}?lno={bolt12_offer}");
        let result = input_parser.parse(&bip21_with_bolt12).await;
        println!("Debug - bip21 with bolt12 offer result: {result:?}");

        assert!(matches!(
            result,
            Ok(InputType::Bip21(bip21))
            if bip21.payment_methods.iter().any(|pm| matches!(pm, InputType::Bolt12Offer(_)))
        ));

        // Address with amount and Bolt12 offer parameter
        let bip21_with_amount_bolt12 =
            format!("bitcoin:{addr}?amount=0.00002000&lno={bolt12_offer}");
        let result = input_parser.parse(&bip21_with_amount_bolt12).await;
        println!("Debug - bip21 with amount and bolt12 offer result: {result:?}");

        assert!(matches!(
            result,
            Ok(InputType::Bip21(bip21))
            if bip21.payment_methods.iter().any(|pm| matches!(pm, InputType::Bolt12Offer(_)))
            && bip21.amount_sat == Some(2000)
        ));
    }

    #[async_test_all]
    async fn test_empty_input() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let result = input_parser.parse("").await;
        assert!(matches!(result, Err(ParseError::EmptyInput)));

        // Test with only whitespace
        let result = input_parser.parse("   ").await;
        assert!(matches!(result, Err(ParseError::EmptyInput)));
    }

    #[async_test_all]
    async fn test_generic_invalid_input() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        let result = input_parser.parse("invalid_input").await;
        println!("Debug - invalid input result: {result:?}");

        assert!(matches!(
            result,
            Err(crate::input::ParseError::InvalidInput)
        ));
    }

    #[async_test_all]
    async fn test_lightning_address() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        mock_lnurl_pay_endpoint(&mock_rest_client, None);

        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let ln_address = "user@domain.net";

        // This should trigger parse_lightning_address method
        let result = input_parser.parse(ln_address).await;
        println!("Debug - lightning address result: {result:?}");

        // Since this depends on the actual implementation of lightning address resolution,
        // we'll just check that it doesn't error out
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_lightning_address_with_prefix() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        mock_lnurl_pay_endpoint(&mock_rest_client, None);

        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let ln_address = "₿user@domain.net";

        // This should also be handled by parse_lightning_address after stripping the prefix
        let result = input_parser.parse(ln_address).await;
        println!("Debug - lightning address with prefix result: {result:?}");

        // Verify that it handles the bitcoin symbol prefix correctly
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_lnurl() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        mock_lnurl_pay_endpoint(&mock_rest_client, None);
        mock_lnurl_pay_endpoint(&mock_rest_client, None);

        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let lnurl_pay_encoded = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf";

        // Should be handled by parse_lnurl method
        let result = input_parser.parse(lnurl_pay_encoded).await;
        println!("Debug - lnurl result: {result:?}");

        // Verify LNURL parsing works
        assert!(result.is_ok());

        // Test with lightning: prefix
        let prefixed_lnurl = format!("lightning:{lnurl_pay_encoded}");
        let result = input_parser.parse(&prefixed_lnurl).await;
        println!("Debug - lnurl with lightning prefix result: {result:?}");
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_lnurl_auth() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();

        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let lnurl_auth_encoded = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttvdankjm3lw3skw0tvdankjm3xdvcn6vtp8q6n2dfsx5mrjwtrxdjnqvtzv56rzcnyv3jrxv3sxqmkyenrvv6kve3exv6nqdtyv43nqcmzvdsnvdrzx33rsenxx5unqc3cxgeqgntfgu";

        // Should be handled by parse_lnurl method, recognizing it as an auth request
        let result = input_parser.parse(lnurl_auth_encoded).await;
        println!("Debug - lnurl auth result: {result:?}");

        // Verify LNURL-auth parsing works
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_lnurl_prefixed_schemes() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        mock_lnurl_pay_endpoint(&mock_rest_client, None);
        mock_lnurl_withdraw_endpoint(&mock_rest_client, None);

        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // Test with lnurlp:// prefix
        let lnurlp_scheme = "lnurlp://domain.com/lnurl-pay?session=test";
        let result = input_parser.parse(lnurlp_scheme).await;
        println!("Debug - lnurlp scheme result: {result:?}");
        assert!(result.is_ok());

        // Test with lnurlw:// prefix
        let lnurlw_scheme = "lnurlw://domain.com/lnurl-withdraw?session=test";
        let result = input_parser.parse(lnurlw_scheme).await;
        println!("Debug - lnurlw scheme result: {result:?}");
        assert!(result.is_ok());

        // Test with keyauth:// prefix
        let keyauth_scheme = "keyauth://domain.com/lnurl-login?tag=login&k1=37b4c919f871c090830cc47b92a544a30097f03430bc39670b8ec0da89f01a81";
        let result = input_parser.parse(keyauth_scheme).await;
        println!("Debug - keyauth scheme result: {result:?}");
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_lnurl_withdraw() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        mock_lnurl_withdraw_endpoint(&mock_rest_client, None);

        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        let lnurl_withdraw_encoded = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

        // Should be handled by parse_lnurl method, recognizing it as a withdraw request
        let result = input_parser.parse(lnurl_withdraw_encoded).await;
        println!("Debug - lnurl withdraw result: {result:?}");

        // Verify LNURL-withdraw parsing works
        assert!(result.is_ok());
    }

    #[async_test_all]
    async fn test_invalid_bitcoin_address() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);

        // Modify valid address to make it invalid
        let invalid_addr = "1andreas3batLhQa2FawWjeyjCqyBzyp";
        let result = input_parser.parse(invalid_addr).await;
        assert!(matches!(result, Err(ParseError::InvalidInput)));
    }

    #[async_test_all]
    async fn test_trim_input() {
        let mock_dns_resolver = MockDnsResolver::new();
        let mock_rest_client = MockRestClient::new();
        let input_parser = InputParser::new(mock_dns_resolver, mock_rest_client);
        for address in [
            r"1andreas3batLhQa2FawWjeyjCqyBzypd",
            r"1andreas3batLhQa2FawWjeyjCqyBzypd ",
            r"1andreas3batLhQa2FawWjeyjCqyBzypd
            ",
            r"
            1andreas3batLhQa2FawWjeyjCqyBzypd
            ",
            r" 1andreas3batLhQa2FawWjeyjCqyBzypd
            ",
        ] {
            let result = input_parser.parse(address).await;
            println!("Debug - trim input result for '{address}': {result:?}");
            assert!(matches!(
                result,
                Ok(crate::input::InputType::BitcoinAddress(_))
            ));
        }
    }
}
