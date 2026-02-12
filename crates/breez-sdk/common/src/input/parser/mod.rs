use std::ops::Not;

use bitcoin::{Address, Denomination, address::NetworkUnchecked};
use lightning::bolt11_invoice::Bolt11InvoiceDescriptionRef;
use regex::Regex;
use spark_wallet::{SparkAddress, SparkAddressPaymentType};
use tracing::{debug, error, warn};
use web_time::UNIX_EPOCH;

use crate::{
    dns::{self, DnsResolver},
    input::{
        Bip21Extra, ExternalInputParser, LnurlRequestDetails, ParseError, PaymentRequestSource,
        SparkAddressDetails, SparkInvoiceDetails,
    },
    lnurl::{auth, error::LnurlError, pay::LnurlPayRequestDetails},
};

use platform_utils::{DefaultHttpClient, HttpClient};

use super::{
    Bip21Details, BitcoinAddressDetails, Bolt11InvoiceDetails, Bolt11RouteHint, Bolt11RouteHintHop,
    Bolt12InvoiceDetails, Bolt12InvoiceRequestDetails, Bolt12Offer, Bolt12OfferBlindedPath,
    Bolt12OfferDetails, InputType, LightningAddressDetails, SilentPaymentAddressDetails,
    error::Bip21Error,
};

const BIP_21_PREFIX: &str = "bitcoin:";
const BIP_353_USER_BITCOIN_PAYMENT_PREFIX: &str = "user._bitcoin-payment";
const LIGHTNING_PREFIX: &str = "lightning:";
const LIGHTNING_PREFIX_LEN: usize = LIGHTNING_PREFIX.len();
const LNURL_HRP: &str = "lnurl";

mod percent_encode;

pub async fn parse(
    input: &str,
    external_input_parsers: Option<Vec<ExternalInputParser>>,
) -> Result<InputType, ParseError> {
    InputParser::new(
        dns::Resolver::new(),
        DefaultHttpClient::default(),
        external_input_parsers,
    )
    .parse(input)
    .await
}

pub fn parse_invoice(input: &str) -> Option<Bolt11InvoiceDetails> {
    parse_bolt11(input, &PaymentRequestSource::default())
}

pub struct InputParser<C, D> {
    http_client: C,
    dns_resolver: D,
    external_input_parsers: Option<Vec<ExternalInputParser>>,
}

impl<C, D> InputParser<C, D>
where
    C: HttpClient + Send + Sync,
    D: DnsResolver + Send + Sync,
{
    pub fn new(
        dns_resolver: D,
        http_client: C,
        external_input_parsers: Option<Vec<ExternalInputParser>>,
    ) -> Self {
        InputParser {
            http_client,
            dns_resolver,
            external_input_parsers,
        }
    }

    pub async fn parse(&self, input: &str) -> Result<InputType, ParseError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ParseError::EmptyInput);
        }

        if let Some(input_type) = self.parse_core(input).await? {
            return Ok(input_type);
        }

        if let Some(input_type) = self.parse_external_input(input).await? {
            return Ok(input_type);
        }

        Err(ParseError::InvalidInput)
    }

    pub async fn parse_core(&self, input: &str) -> Result<Option<InputType>, ParseError> {
        if input.contains('@') {
            if let Some(lightning_address) = self.parse_lightning_address(input).await {
                return Ok(Some(InputType::LightningAddress(lightning_address)));
            }

            if let Some(bip_21) = self.parse_bip_353(input).await? {
                return Ok(Some(InputType::Bip21(bip_21)));
            }
        }

        if has_bip_21_prefix(input) {
            let source = PaymentRequestSource {
                bip_21_uri: Some(input.to_string()),
                bip_353_address: None,
            };
            if let Some(bip_21) = parse_bip_21(input, &source)? {
                return Ok(Some(InputType::Bip21(bip_21)));
            }
        }

        let source = PaymentRequestSource::default();
        if let Some(input_type) = self.parse_lightning(input, &source).await? {
            return Ok(Some(input_type));
        }

        if let Some(input_type) = parse_spark_address(input, &source) {
            return Ok(Some(input_type));
        }

        if let Some(input_type) = parse_bitcoin(input, &source) {
            return Ok(Some(input_type));
        }

        Ok(None)
    }

    async fn parse_bip_353(&self, input: &str) -> Result<Option<Bip21Details>, Bip21Error> {
        // BIP-353 addresses may have a ₿ prefix, so strip it if present
        let Some((local_part, domain)) = input.strip_prefix('₿').unwrap_or(input).split_once('@')
        else {
            return Ok(None);
        };

        // Validate both parts are within the DNS label size limit.
        // See <https://datatracker.ietf.org/doc/html/rfc1035#section-2.3.4>
        if local_part.len() > 63 {
            return Ok(None);
        }

        // Domain can contain multiple labels - validate each one
        if domain
            .split('.')
            .any(|label| label.is_empty() || label.len() > 63)
        {
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

        // Use http:// for Tor or local domains (latter being commonly used for testing)
        let scheme = if has_extension(&domain, "onion")
            || domain.starts_with("127.0.0.1")
            || domain.starts_with("localhost")
        {
            "http://"
        } else {
            "https://"
        };

        let Ok(url) = url::Url::parse(&format!("{scheme}{domain}/.well-known/lnurlp/{user}"))
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

        let Ok(parsed_url) = url::Url::parse(&input) else {
            return Ok(None);
        };

        let host = match parsed_url.host() {
            Some(domain) => domain.to_string(),
            None => return Ok(None), // TODO: log or return error.
        };

        let mut url = parsed_url.clone();
        match parsed_url.scheme() {
            "http" => {
                // Allow http for .onion domains and local domains (for testing)
                if !has_extension(&host, "onion") && !is_local_domain(&host) {
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
                    url =
                        url::Url::parse(&replace_prefix(&input, scheme, "http")).map_err(|_| {
                            LnurlError::General(
                                "failed to rewrite lnurl scheme to http".to_string(),
                            )
                        })?;
                } else {
                    url = url::Url::parse(&replace_prefix(&input, scheme, "https")).map_err(
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
        url: &url::Url,
        _source: &PaymentRequestSource,
    ) -> Result<InputType, LnurlError> {
        if let Some(query) = url.query()
            && query.contains("tag=login")
        {
            let data = auth::validate_request(url)?;
            return Ok(InputType::LnurlAuth(data));
        }

        let response = self.http_client.get(url.to_string(), None).await?;
        let lnurl_data: LnurlRequestDetails = response.json()?;
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

    async fn parse_external_input(&self, input: &str) -> Result<Option<InputType>, ParseError> {
        let Some(external_input_parsers) = &self.external_input_parsers else {
            return Ok(None);
        };

        for parser in external_input_parsers {
            // Check regex
            let re = Regex::new(&parser.input_regex)?;
            if re.is_match(input).not() {
                continue;
            }

            // Build URL
            let urlsafe_input = percent_encode::encode(input);
            let parser_url = parser.parser_url.replacen("<input>", &urlsafe_input, 1);

            // Make request
            let response = self.http_client.get(parser_url.clone(), None).await?;
            let body = &response.body;

            // Try to parse as LnurlRequestDetails
            if let Ok(lnurl_data) = response.json::<LnurlRequestDetails>() {
                let domain = url::Url::parse(&parser_url)
                    .ok()
                    .and_then(|url| url.host_str().map(ToString::to_string))
                    .unwrap_or_default();
                let input_type = lnurl_data.try_into()?;
                let input_type = match input_type {
                    // Modify the LnUrlPay payload by adding the domain of the LNURL endpoint
                    InputType::LnurlPay(pay_request) => {
                        InputType::LnurlPay(LnurlPayRequestDetails {
                            domain,
                            ..pay_request
                        })
                    }
                    _ => input_type,
                };
                return Ok(Some(input_type));
            }

            // Check other input types
            if let Ok(input_type) = self.parse_core(body).await {
                return Ok(input_type);
            }
        }

        Ok(None)
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

/// Check if the domain is a local domain (for testing purposes)
fn is_local_domain(host: &str) -> bool {
    host.starts_with("127.0.0.1") || host.starts_with("localhost")
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
            bip_21.label = Some(
                percent_encode::decode(value)
                    .map_err(Bip21Error::invalid_parameter_func("label"))?,
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
            bip_21.message = Some(
                percent_encode::decode(value)
                    .map_err(Bip21Error::invalid_parameter_func("message"))?,
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

pub fn parse_spark_address(input: &str, source: &PaymentRequestSource) -> Option<InputType> {
    if let Ok(spark_address) = input.parse::<SparkAddress>() {
        let identity_public_key = spark_address.identity_public_key.to_string();
        let network = spark_address.network.into();

        if spark_address.is_invoice() {
            let invoice_fields = spark_address.spark_invoice_fields?;

            let payment_type = invoice_fields.payment_type?;

            let amount = match &payment_type {
                SparkAddressPaymentType::TokensPayment(tp) => tp.amount,
                SparkAddressPaymentType::SatsPayment(sp) => sp.amount.map(Into::into),
            };

            let token_identifier = match &payment_type {
                SparkAddressPaymentType::TokensPayment(tp) => {
                    let Some(token_identifier) = &tp.token_identifier else {
                        warn!(
                            "Tried parsing Spark token invoice without token identifier: {input}"
                        );
                        return None;
                    };
                    Some(token_identifier.clone())
                }
                SparkAddressPaymentType::SatsPayment(_) => None,
            };

            let Ok(expiry_time_duration) = invoice_fields
                .expiry_time
                .map(|e| e.duration_since(UNIX_EPOCH))
                .transpose()
            else {
                return None;
            };
            let expiry_time = expiry_time_duration.map(|e| e.as_secs());

            return Some(InputType::SparkInvoice(SparkInvoiceDetails {
                invoice: input.to_string(),
                identity_public_key,
                network,
                amount,
                token_identifier,
                expiry_time,
                description: invoice_fields.memo,
                sender_public_key: invoice_fields.sender_public_key.map(|e| e.to_string()),
            }));
        }

        return Some(InputType::SparkAddress(SparkAddressDetails {
            address: input.to_string(),
            identity_public_key,
            network,
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

#[cfg(test)]
mod tests;
