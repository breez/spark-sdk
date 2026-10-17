use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex as StdMutex};

use bitcoin::{
    Address, Network, OutPoint, Txid,
    address::NetworkUnchecked,
    bip32::{DerivationPath, Xpriv},
    key::Secp256k1,
    secp256k1::{self, XOnlyPublicKey},
};
use sqlx::{PgConnection, PgPool};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::info;

use crate::chain::{ChainRepository, ChainRepositoryError};

/// Confirmations a conflicting spend needs before the stored transaction it beat
/// releases its other coins.
pub const CONFLICT_CONFIRMATIONS: i64 = 6;

#[derive(Error, Debug)]
pub enum OnchainWalletError {
    #[error("key derivation error: {0}")]
    KeyDerivation(String),

    #[error("chain repository error: {0}")]
    ChainRepository(#[from] ChainRepositoryError),

    #[error("persistence error: {0}")]
    Persistence(String),

    #[error("insufficient funds: need {need} sats, have {have} sats")]
    InsufficientFunds { need: u64, have: u64 },
}

/// Derives every address, change too, as BIP86 P2TR on `m/86'/coin'/0'/0/i` from
/// the seed.
pub struct OnchainWallet<R: ChainRepository> {
    xprv: Xpriv,
    network: Network,
    chain_repository: Arc<R>,
    pgpool: Arc<PgPool>,
    secp: Secp256k1<secp256k1::All>,
    selection: Mutex<()>,
    held: Arc<StdMutex<HashSet<OutPoint>>>,
}

#[derive(Debug, Clone)]
pub struct Utxo {
    pub outpoint: OutPoint,
    pub value: u64,
    pub block_height: u64,
    pub address: Address,
}

/// Coins taken for a transaction being built, kept out of every other selection
/// until this is dropped. Hold it until the transaction's spends are recorded.
pub struct HeldCoins {
    held: Arc<StdMutex<HashSet<OutPoint>>>,
    outpoints: Vec<OutPoint>,
}

impl Drop for HeldCoins {
    fn drop(&mut self) {
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for outpoint in &self.outpoints {
            held.remove(outpoint);
        }
    }
}

impl<R: ChainRepository> OnchainWallet<R> {
    pub fn new(
        seed: &[u8],
        network: Network,
        chain_repository: Arc<R>,
        pgpool: Arc<PgPool>,
    ) -> Result<Self, OnchainWalletError> {
        let xprv = Xpriv::new_master(network, seed)
            .map_err(|e| OnchainWalletError::KeyDerivation(e.to_string()))?;

        Ok(Self {
            xprv,
            network,
            chain_repository,
            pgpool,
            secp: Secp256k1::new(),
            selection: Mutex::new(()),
            held: Arc::new(StdMutex::new(HashSet::new())),
        })
    }

    pub async fn next_address(&self) -> Result<(Address, u32), OnchainWalletError> {
        let mut addresses = self.next_addresses(1).await?;
        addresses.pop().ok_or_else(|| {
            OnchainWalletError::Persistence("the wallet made no address".to_string())
        })
    }

    pub async fn next_addresses(
        &self,
        count: u32,
    ) -> Result<Vec<(Address, u32)>, OnchainWalletError> {
        let first = self.take_address_indexes(count).await?;
        let mut addresses = Vec::new();
        for index in first..first.saturating_add(count) {
            addresses.push((self.derive_address(index)?, index));
        }
        let texts: Vec<String> = addresses.iter().map(|(a, _)| a.to_string()).collect();
        let indexes: Vec<i64> = addresses.iter().map(|(_, i)| i64::from(*i)).collect();
        sqlx::query(
            "INSERT INTO brz_ssp_onchain_wallet_addresses (address, derivation_index)
             SELECT * FROM UNNEST($1::text[], $2::bigint[])
             ON CONFLICT (address) DO NOTHING",
        )
        .bind(&texts)
        .bind(&indexes)
        .execute(self.pgpool.as_ref())
        .await
        .map_err(|e| OnchainWalletError::Persistence(e.to_string()))?;
        let watched: Vec<Address> = addresses.iter().map(|(a, _)| a.clone()).collect();
        self.chain_repository.add_watch_addresses(&watched).await?;
        for (address, index) in &addresses {
            info!(%address, index, "registered new onchain wallet address");
        }
        Ok(addresses)
    }

    /// The wallet's confirmed unspent outputs, less those a build in progress holds
    /// and those a recorded spend withholds. A recorded spend stops withholding once
    /// another spend of one of its coins has [`CONFLICT_CONFIRMATIONS`].
    pub async fn list_utxos(&self) -> Result<Vec<Utxo>, OnchainWalletError> {
        // Read before the spends: a build records its spends before it releases
        // the coins it holds.
        let held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let rows: Vec<(String, String, i32, i64, i64)> = sqlx::query_as(
            r"SELECT o.address, o.tx_id, o.output_index, o.amount, b.height
               FROM brz_ssp_chain_tx_outputs o
               INNER JOIN brz_ssp_onchain_wallet_addresses a ON a.address = o.address
               INNER JOIN brz_ssp_chain_tx_blocks tb ON tb.tx_id = o.tx_id
               INNER JOIN brz_ssp_chain_blocks b ON b.block_hash = tb.block_hash
               WHERE NOT EXISTS (
                   SELECT 1
                   FROM brz_ssp_chain_tx_inputs i
                   INNER JOIN brz_ssp_chain_tx_blocks itb ON itb.tx_id = i.spending_tx_id
                   WHERE i.tx_id = o.tx_id AND i.output_index = o.output_index)
               AND NOT EXISTS (
                   SELECT 1
                   FROM brz_ssp_onchain_wallet_spends s
                   WHERE s.tx_id = o.tx_id AND s.output_index = o.output_index
                   AND NOT EXISTS (
                       SELECT 1
                       FROM brz_ssp_onchain_wallet_spends lost
                       INNER JOIN brz_ssp_chain_tx_inputs ci
                           ON ci.tx_id = lost.tx_id
                           AND ci.output_index = lost.output_index
                           AND ci.spending_tx_id <> lost.spending_txid
                       INNER JOIN brz_ssp_chain_tx_blocks ctb ON ctb.tx_id = ci.spending_tx_id
                       INNER JOIN brz_ssp_chain_blocks cb ON cb.block_hash = ctb.block_hash
                       WHERE lost.spending_txid = s.spending_txid
                       AND (SELECT MAX(height) FROM brz_ssp_chain_blocks) - cb.height + 1 >= $1))",
        )
        .bind(CONFLICT_CONFIRMATIONS)
        .fetch_all(self.pgpool.as_ref())
        .await
        .map_err(|e| OnchainWalletError::Persistence(e.to_string()))?;
        rows.into_iter()
            .map(|(address, tx_id, output_index, amount, height)| {
                Ok(Utxo {
                    outpoint: outpoint(&tx_id, output_index)?,
                    value: amount.cast_unsigned(),
                    block_height: height.cast_unsigned(),
                    address: self.parse_address(&address)?,
                })
            })
            .filter(|utxo| {
                utxo.as_ref()
                    .map_or(true, |utxo| !held.contains(&utxo.outpoint))
            })
            .collect()
    }

    /// The wallet's outputs at `outpoints`, spent or not.
    pub async fn outputs_at(
        &self,
        outpoints: &[OutPoint],
    ) -> Result<Vec<Utxo>, OnchainWalletError> {
        let tx_ids: Vec<String> = outpoints.iter().map(|o| o.txid.to_string()).collect();
        let indices: Vec<i32> = outpoints.iter().map(|o| o.vout.cast_signed()).collect();
        let rows: Vec<(String, String, i32, i64)> = sqlx::query_as(
            r"SELECT o.address, o.tx_id, o.output_index, o.amount
               FROM brz_ssp_chain_tx_outputs o
               INNER JOIN brz_ssp_onchain_wallet_addresses a ON a.address = o.address
               INNER JOIN UNNEST($1::text[], $2::int[]) AS p(tx_id, output_index)
                   ON p.tx_id = o.tx_id AND p.output_index = o.output_index",
        )
        .bind(&tx_ids)
        .bind(&indices)
        .fetch_all(self.pgpool.as_ref())
        .await
        .map_err(|e| OnchainWalletError::Persistence(e.to_string()))?;
        rows.into_iter()
            .map(|(address, tx_id, output_index, amount)| {
                Ok(Utxo {
                    outpoint: outpoint(&tx_id, output_index)?,
                    value: amount.cast_unsigned(),
                    block_height: 0,
                    address: self.parse_address(&address)?,
                })
            })
            .collect()
    }

    fn parse_address(&self, address: &str) -> Result<Address, OnchainWalletError> {
        address
            .parse::<Address<NetworkUnchecked>>()
            .and_then(|a| a.require_network(self.network))
            .map_err(|e| OnchainWalletError::Persistence(format!("bad address {address}: {e}")))
    }

    /// Takes coins, largest first, until they cover `need(inputs)`: what a
    /// transaction spending that many inputs has to fund, fee included.
    pub async fn select_coins(
        &self,
        need: &(dyn Fn(usize) -> u64 + Send + Sync),
    ) -> Result<(Vec<Utxo>, HeldCoins), OnchainWalletError> {
        let _selection = self.selection.lock().await;
        let mut utxos = self.list_utxos().await?;
        utxos.sort_by_key(|u| std::cmp::Reverse(u.value));

        let mut selected = Vec::new();
        let mut total: u64 = 0;
        for utxo in utxos {
            if total >= need(selected.len()) && !selected.is_empty() {
                break;
            }
            total = total.saturating_add(utxo.value);
            selected.push(utxo);
        }
        let required = need(selected.len());
        if selected.is_empty() || total < required {
            return Err(OnchainWalletError::InsufficientFunds {
                need: required,
                have: total,
            });
        }

        let outpoints: Vec<OutPoint> = selected.iter().map(|u| u.outpoint).collect();
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(outpoints.iter().copied());
        Ok((
            selected,
            HeldCoins {
                held: Arc::clone(&self.held),
                outpoints,
            },
        ))
    }

    fn derivation_path(&self, index: u32) -> Result<DerivationPath, OnchainWalletError> {
        let coin = u32::from(self.network != Network::Bitcoin);
        DerivationPath::from_str(&format!("m/86'/{coin}'/0'/0/{index}"))
            .map_err(|e| OnchainWalletError::KeyDerivation(e.to_string()))
    }

    fn derive_secret_key(&self, index: u32) -> Result<secp256k1::SecretKey, OnchainWalletError> {
        let derived = self
            .xprv
            .derive_priv(&self.secp, &self.derivation_path(index)?)
            .map_err(|e| OnchainWalletError::KeyDerivation(e.to_string()))?;
        Ok(derived.private_key)
    }

    fn derive_address(&self, index: u32) -> Result<Address, OnchainWalletError> {
        let secret_key = self.derive_secret_key(index)?;
        let (xonly, _parity) = XOnlyPublicKey::from_keypair(&secret_key.keypair(&self.secp));
        Ok(Address::p2tr(&self.secp, xonly, None, self.network))
    }

    /// Takes `count` address indexes in a row, and returns the first.
    async fn take_address_indexes(&self, count: u32) -> Result<u32, OnchainWalletError> {
        let (next,): (String,) = sqlx::query_as(
            "INSERT INTO brz_ssp_onchain_wallet_meta (key, value)
             VALUES ('next_address_index', $1::BIGINT::TEXT)
             ON CONFLICT (key) DO UPDATE
             SET value = (brz_ssp_onchain_wallet_meta.value::BIGINT + $1::BIGINT)::TEXT
             RETURNING value",
        )
        .bind(i64::from(count))
        .fetch_one(self.pgpool.as_ref())
        .await
        .map_err(|e| OnchainWalletError::Persistence(e.to_string()))?;
        next.parse::<u32>()
            .ok()
            .and_then(|next| next.checked_sub(count))
            .ok_or_else(|| OnchainWalletError::Persistence(format!("bad address index {next}")))
    }

    pub fn network(&self) -> Network {
        self.network
    }

    /// The untweaked taproot secret key of each coin's address.
    pub async fn keys_for(
        &self,
        coins: &[Utxo],
    ) -> Result<HashMap<Address, secp256k1::SecretKey>, OnchainWalletError> {
        let addresses: HashSet<&Address> = coins.iter().map(|coin| &coin.address).collect();
        let addresses: Vec<Address> = addresses.into_iter().cloned().collect();
        self.keys_for_addresses(&addresses).await
    }

    /// The untweaked taproot secret key of each of `addresses`. Fails for an
    /// address this wallet did not derive.
    pub async fn keys_for_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<HashMap<Address, secp256k1::SecretKey>, OnchainWalletError> {
        let texts: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT address, derivation_index FROM brz_ssp_onchain_wallet_addresses
             WHERE address = ANY($1)",
        )
        .bind(&texts)
        .fetch_all(self.pgpool.as_ref())
        .await
        .map_err(|e| OnchainWalletError::Persistence(e.to_string()))?;
        let indexes: HashMap<String, i64> = rows.into_iter().collect();
        let mut keys = HashMap::with_capacity(addresses.len());
        for (address, text) in addresses.iter().zip(&texts) {
            let Some(index) = indexes.get(text) else {
                return Err(OnchainWalletError::KeyDerivation(format!(
                    "address {address} is not one this wallet derived"
                )));
            };
            let index = u32::try_from(*index).map_err(|_| {
                OnchainWalletError::Persistence(format!("derivation index {index} out of range"))
            })?;
            keys.insert(address.clone(), self.derive_secret_key(index)?);
        }
        Ok(keys)
    }

    /// Watches `addresses` without making them the wallet's own.
    pub async fn register_watch_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<(), OnchainWalletError> {
        self.chain_repository.add_watch_addresses(addresses).await?;
        Ok(())
    }
}

/// Keeps `outpoints` out of [`OnchainWallet::list_utxos`]. Takes a connection so
/// the spends can commit with the database transaction that stores `spending_txid`.
pub async fn record_spends(
    db: &mut PgConnection,
    spending_txid: &Txid,
    outpoints: &[OutPoint],
) -> Result<(), sqlx::Error> {
    let tx_ids: Vec<String> = outpoints.iter().map(|o| o.txid.to_string()).collect();
    let indices: Vec<i32> = outpoints.iter().map(|o| o.vout.cast_signed()).collect();
    sqlx::query(
        "INSERT INTO brz_ssp_onchain_wallet_spends (tx_id, output_index, spending_txid)
         SELECT p.tx_id, p.output_index, $3
         FROM UNNEST($1::text[], $2::int[]) AS p(tx_id, output_index)
         ON CONFLICT DO NOTHING",
    )
    .bind(&tx_ids)
    .bind(&indices)
    .bind(spending_txid.to_string())
    .execute(db)
    .await?;
    Ok(())
}

fn outpoint(tx_id: &str, output_index: i32) -> Result<OutPoint, OnchainWalletError> {
    Ok(OutPoint {
        txid: tx_id
            .parse()
            .map_err(|e| OnchainWalletError::Persistence(format!("bad txid {tx_id}: {e}")))?,
        vout: output_index.cast_unsigned(),
    })
}

#[async_trait::async_trait]
impl<R: ChainRepository + Send + Sync + 'static> crate::coop_exit::CoopExitOnchainWallet
    for OnchainWallet<R>
{
    fn network(&self) -> Network {
        OnchainWallet::network(self)
    }

    async fn select_coins(
        &self,
        need: &(dyn Fn(usize) -> u64 + Send + Sync),
    ) -> Result<(Vec<Utxo>, HeldCoins), Box<dyn std::error::Error + Send + Sync>> {
        Ok(OnchainWallet::select_coins(self, need).await?)
    }

    async fn next_addresses(
        &self,
        count: u32,
    ) -> Result<Vec<Address>, Box<dyn std::error::Error + Send + Sync>> {
        let addresses = OnchainWallet::next_addresses(self, count).await?;
        Ok(addresses.into_iter().map(|(address, _)| address).collect())
    }

    async fn keys_for_addresses(
        &self,
        addresses: &[Address],
    ) -> Result<HashMap<Address, secp256k1::SecretKey>, Box<dyn std::error::Error + Send + Sync>>
    {
        Ok(OnchainWallet::keys_for_addresses(self, addresses).await?)
    }
}
