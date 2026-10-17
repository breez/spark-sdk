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
        self.select_at_most(usize::MAX, need).await
    }

    pub async fn select_at_most(
        &self,
        most: usize,
        need: &(dyn Fn(usize) -> u64 + Send + Sync),
    ) -> Result<(Vec<Utxo>, HeldCoins), OnchainWalletError> {
        let _selection = self.selection.lock().await;
        let mut utxos = self.list_utxos().await?;
        utxos.sort_by_key(|u| std::cmp::Reverse(u.value));

        let mut selected = Vec::new();
        let mut total: u64 = 0;
        for utxo in utxos {
            if (total >= need(selected.len()) && !selected.is_empty()) || selected.len() >= most {
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

/// Undoes [`record_spends`] for each of `spending_txids`.
pub async fn forget_spends(
    db: &mut PgConnection,
    spending_txids: &[Txid],
) -> Result<(), sqlx::Error> {
    let txids: Vec<String> = spending_txids.iter().map(ToString::to_string).collect();
    sqlx::query("DELETE FROM brz_ssp_onchain_wallet_spends WHERE spending_txid = ANY($1)")
        .bind(&txids)
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

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use bitcoin::secp256k1::SecretKey;
    use bitcoin::{Amount, BlockHash, TxOut};

    use super::*;
    use crate::chain::{AddressUtxo, BlockHeader, ChainRepository as _, SpentTxo, Txo};
    use crate::postgresql::ChainRepository;
    use crate::postgresql::testing::TestDatabase;

    const SEED: [u8; 32] = [7; 32];

    struct Fixture {
        wallet: Arc<OnchainWallet<ChainRepository>>,
        chain: Arc<ChainRepository>,
        pool: Arc<PgPool>,
        _database: TestDatabase,
    }

    impl Fixture {
        async fn new() -> Self {
            Self::with(&SEED, Network::Regtest).await
        }

        async fn with(seed: &[u8], network: Network) -> Self {
            let database = TestDatabase::new().await;
            let pool = Arc::clone(&database.pool);
            let chain = Arc::new(ChainRepository::new(Arc::clone(&pool), network));
            let wallet =
                OnchainWallet::new(seed, network, Arc::clone(&chain), Arc::clone(&pool)).unwrap();
            Self {
                wallet: Arc::new(wallet),
                chain,
                pool,
                _database: database,
            }
        }

        async fn addresses(&self, count: u32) -> Vec<Address> {
            let addresses = self.wallet.next_addresses(count).await.unwrap();
            addresses.into_iter().map(|(address, _)| address).collect()
        }

        async fn add_block(
            &self,
            height: u8,
            outputs: &[(OutPoint, &Address, u64)],
            spends: &[(OutPoint, Txid)],
        ) {
            let outputs: Vec<AddressUtxo> = outputs
                .iter()
                .map(|(outpoint, address, value)| AddressUtxo {
                    address: (*address).clone(),
                    utxo: Txo {
                        block_height: u64::from(height),
                        outpoint: *outpoint,
                        tx_out: TxOut {
                            value: Amount::from_sat(*value),
                            script_pubkey: address.script_pubkey(),
                        },
                    },
                })
                .collect();
            let spends: Vec<SpentTxo> = spends
                .iter()
                .zip(0..)
                .map(|((outpoint, spending_tx), spending_input_index)| SpentTxo {
                    outpoint: *outpoint,
                    spending_tx: *spending_tx,
                    spending_input_index,
                })
                .collect();
            let block = BlockHeader {
                hash: block_hash(height),
                height: u64::from(height),
            };
            self.chain
                .add_block(&block, &outputs, &spends)
                .await
                .unwrap();
        }

        async fn coin(&self, height: u8, value: u64) -> OutPoint {
            let (address, _) = self.wallet.next_address().await.unwrap();
            let coin = output(height);
            self.add_block(height, &[(coin, &address, value)], &[])
                .await;
            coin
        }

        async fn coins(&self) -> Vec<OutPoint> {
            let coins = self.wallet.list_utxos().await.unwrap();
            let mut coins: Vec<OutPoint> = coins.iter().map(|coin| coin.outpoint).collect();
            coins.sort();
            coins
        }

        async fn record_spends(&self, spending_txid: Txid, outpoints: &[OutPoint]) {
            let mut db = self.pool.acquire().await.unwrap();
            super::record_spends(&mut db, &spending_txid, outpoints)
                .await
                .unwrap();
        }

        async fn forget_spends(&self, spending_txids: &[Txid]) {
            let mut db = self.pool.acquire().await.unwrap();
            super::forget_spends(&mut db, spending_txids).await.unwrap();
        }
    }

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_slice(&[byte; 32]).unwrap()
    }

    fn taproot_address(key: &SecretKey) -> Address {
        let secp = Secp256k1::new();
        let (internal_key, _) = key.public_key(&secp).x_only_public_key();
        Address::p2tr(&secp, internal_key, None, Network::Regtest)
    }

    fn block_hash(height: u8) -> BlockHash {
        BlockHash::from_byte_array([height; 32])
    }

    fn txid(byte: u8) -> Txid {
        Txid::from_byte_array([byte; 32])
    }

    fn output(byte: u8) -> OutPoint {
        OutPoint::new(txid(byte), 0)
    }

    fn in_order(outputs: Vec<Utxo>) -> Vec<(OutPoint, u64, Address)> {
        let mut outputs: Vec<_> = outputs
            .into_iter()
            .map(|output| (output.outpoint, output.value, output.address))
            .collect();
        outputs.sort();
        outputs
    }

    #[tokio::test]
    async fn new_addresses_take_consecutive_derivation_indexes() {
        let fixture = Fixture::new().await;

        let addresses = fixture.wallet.next_addresses(3).await.unwrap();

        let indexes: Vec<u32> = addresses.iter().map(|(_, index)| *index).collect();
        assert_eq!(indexes, [0, 1, 2]);
        let distinct: HashSet<&Address> = addresses.iter().map(|(address, _)| address).collect();
        assert_eq!(distinct.len(), 3);
    }

    #[tokio::test]
    async fn the_indexes_of_a_later_call_follow_those_of_an_earlier_one() {
        let fixture = Fixture::new().await;
        fixture.wallet.next_addresses(2).await.unwrap();

        let addresses = fixture.wallet.next_addresses(3).await.unwrap();

        let indexes: Vec<u32> = addresses.iter().map(|(_, index)| *index).collect();
        assert_eq!(indexes, [2, 3, 4]);
    }

    #[tokio::test]
    async fn a_single_new_address_takes_one_index() {
        let fixture = Fixture::new().await;

        let (first, first_index) = fixture.wallet.next_address().await.unwrap();
        let one = fixture.wallet.next_addresses(1).await.unwrap();
        let (third, third_index) = fixture.wallet.next_address().await.unwrap();

        assert_eq!(first_index, 0);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].1, 1);
        assert_eq!(third_index, 2);
        let distinct = HashSet::from([&first, &one[0].0, &third]);
        assert_eq!(distinct.len(), 3);
    }

    #[tokio::test]
    async fn no_new_addresses_take_no_index() {
        let fixture = Fixture::new().await;

        assert!(fixture.wallet.next_addresses(0).await.unwrap().is_empty());
        let (_, first_index) = fixture.wallet.next_address().await.unwrap();
        assert!(fixture.wallet.next_addresses(0).await.unwrap().is_empty());
        let (_, second_index) = fixture.wallet.next_address().await.unwrap();

        assert_eq!((first_index, second_index), (0, 1));
    }

    #[tokio::test]
    async fn addresses_taken_at_the_same_time_get_distinct_indexes() {
        let fixture = Fixture::new().await;

        let calls: Vec<_> = (0..10)
            .map(|_| {
                let wallet = Arc::clone(&fixture.wallet);
                tokio::spawn(async move { wallet.next_addresses(3).await.unwrap() })
            })
            .collect();
        let mut indexes = Vec::new();
        for call in calls {
            let addresses = call.await.unwrap();
            indexes.extend(addresses.into_iter().map(|(_, index)| index));
        }

        indexes.sort_unstable();
        assert_eq!(indexes, (0..30).collect::<Vec<u32>>());
    }

    #[tokio::test]
    async fn a_wallet_started_again_goes_on_from_the_stored_addresses() {
        let fixture = Fixture::new().await;
        let earlier = fixture.addresses(2).await;
        let chain = Arc::clone(&fixture.chain);
        let pool = Arc::clone(&fixture.pool);

        let restarted = OnchainWallet::new(&SEED, Network::Regtest, chain, pool).unwrap();

        let (_, index) = restarted.next_address().await.unwrap();
        assert_eq!(index, 2);
        let keys = restarted.keys_for_addresses(&earlier).await.unwrap();
        for address in &earlier {
            assert_eq!(taproot_address(&keys[address]), *address);
        }
    }

    #[tokio::test]
    async fn the_addresses_of_a_seed_are_its_bip86_addresses() {
        // The seed and the first two receiving addresses of the test vectors in
        // BIP 86.
        let seed = hex::decode(
            "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1\
             9a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4",
        )
        .unwrap();
        let fixture = Fixture::with(&seed, Network::Bitcoin).await;

        let addresses = fixture.addresses(2).await;

        let addresses: Vec<String> = addresses.iter().map(ToString::to_string).collect();
        assert_eq!(
            addresses,
            [
                "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr",
                "bc1p4qhjn9zdvkux4e44uhx8tc55attvtyu358kutcqkudyccelu0was9fqzwh",
            ]
        );
    }

    #[tokio::test]
    async fn the_chain_record_watches_every_new_address() {
        let fixture = Fixture::new().await;
        let not_the_wallets = taproot_address(&key(9));

        let mut addresses = fixture.addresses(3).await;

        addresses.push(not_the_wallets);
        let watched = fixture
            .chain
            .filter_watch_addresses(&addresses)
            .await
            .unwrap();
        let watched: HashSet<&Address> = watched.iter().collect();
        assert_eq!(watched, addresses[..3].iter().collect());
    }

    #[tokio::test]
    async fn each_address_gets_the_key_it_was_derived_from() {
        let fixture = Fixture::new().await;
        let addresses = fixture.addresses(3).await;

        let keys = fixture.wallet.keys_for_addresses(&addresses).await.unwrap();

        assert_eq!(keys.len(), 3);
        for address in &addresses {
            assert_eq!(taproot_address(&keys[address]), *address);
        }
    }

    #[tokio::test]
    async fn only_the_given_addresses_get_a_key() {
        let fixture = Fixture::new().await;
        let addresses = fixture.addresses(3).await;
        let wallet = &fixture.wallet;

        assert!(wallet.keys_for_addresses(&[]).await.unwrap().is_empty());
        assert!(wallet.keys_for(&[]).await.unwrap().is_empty());
        let keys = wallet.keys_for_addresses(&addresses[1..2]).await.unwrap();
        assert_eq!(keys.keys().collect::<Vec<_>>(), [&addresses[1]]);
    }

    #[tokio::test]
    async fn an_address_the_wallet_did_not_derive_gets_no_key() {
        let fixture = Fixture::new().await;
        let own = fixture.addresses(1).await.remove(0);
        let not_the_wallets = taproot_address(&key(9));
        let wallet = &fixture.wallet;

        let alone = wallet
            .keys_for_addresses(std::slice::from_ref(&not_the_wallets))
            .await;
        let among_its_own = wallet.keys_for_addresses(&[own, not_the_wallets]).await;

        assert!(matches!(alone, Err(OnchainWalletError::KeyDerivation(_))));
        assert!(matches!(
            among_its_own,
            Err(OnchainWalletError::KeyDerivation(_))
        ));
    }

    #[tokio::test]
    async fn an_address_given_twice_gets_its_key_once() {
        let fixture = Fixture::new().await;
        let address = fixture.addresses(1).await.remove(0);

        let keys = fixture
            .wallet
            .keys_for_addresses(&[address.clone(), address.clone()])
            .await
            .unwrap();

        assert_eq!(keys.len(), 1);
        assert_eq!(taproot_address(&keys[&address]), address);
    }

    #[tokio::test]
    async fn the_coins_at_one_address_share_its_key() {
        let fixture = Fixture::new().await;
        let addresses = fixture.addresses(2).await;
        fixture
            .add_block(
                1,
                &[
                    (output(1), &addresses[0], 5_000),
                    (output(2), &addresses[0], 6_000),
                    (output(3), &addresses[1], 7_000),
                ],
                &[],
            )
            .await;
        let coins = fixture.wallet.list_utxos().await.unwrap();
        assert_eq!(coins.len(), 3);

        let keys = fixture.wallet.keys_for(&coins).await.unwrap();

        assert_eq!(keys.len(), 2);
        for address in &addresses {
            assert_eq!(taproot_address(&keys[address]), *address);
        }
    }

    #[tokio::test]
    async fn a_coin_at_an_address_the_wallet_did_not_derive_gets_no_key() {
        let fixture = Fixture::new().await;
        let coin = Utxo {
            outpoint: output(1),
            value: 5_000,
            block_height: 1,
            address: taproot_address(&key(9)),
        };

        let keys = fixture.wallet.keys_for(&[coin]).await;

        assert!(matches!(keys, Err(OnchainWalletError::KeyDerivation(_))));
    }

    #[tokio::test]
    async fn an_address_registered_for_watching_only_is_not_the_wallets_own() {
        let fixture = Fixture::new().await;
        let address = taproot_address(&key(9));
        let only = std::slice::from_ref(&address);

        fixture.wallet.register_watch_addresses(only).await.unwrap();
        fixture
            .add_block(1, &[(output(1), &address, 5_000)], &[])
            .await;

        let watched = fixture.chain.filter_watch_addresses(only).await.unwrap();
        assert_eq!(watched, only);
        assert!(fixture.wallet.keys_for_addresses(only).await.is_err());
        assert!(fixture.coins().await.is_empty());
    }

    #[tokio::test]
    async fn an_output_at_an_address_of_the_wallets_in_a_block_is_a_coin() {
        let fixture = Fixture::new().await;
        let addresses = fixture.addresses(2).await;
        fixture
            .add_block(3, &[(output(1), &addresses[0], 5_000)], &[])
            .await;
        fixture
            .add_block(4, &[(output(2), &addresses[1], 6_000)], &[])
            .await;

        let mut coins = fixture.wallet.list_utxos().await.unwrap();

        coins.sort_by_key(|coin| coin.outpoint);
        let heights: Vec<u64> = coins.iter().map(|coin| coin.block_height).collect();
        assert_eq!(heights, [3, 4]);
        assert_eq!(
            in_order(coins),
            [
                (output(1), 5_000, addresses[0].clone()),
                (output(2), 6_000, addresses[1].clone()),
            ]
        );
    }

    #[tokio::test]
    async fn a_coin_spent_on_chain_is_no_longer_listed() {
        let fixture = Fixture::new().await;
        let (spent, unspent) = (fixture.coin(1, 5_000).await, fixture.coin(2, 6_000).await);

        fixture.add_block(3, &[], &[(spent, txid(9))]).await;

        assert_eq!(fixture.coins().await, [unspent]);
    }

    #[tokio::test]
    async fn a_coin_is_listed_again_once_the_block_of_its_spend_is_undone() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1, 5_000).await;
        fixture.add_block(2, &[], &[(coin, txid(9))]).await;

        fixture.chain.undo_block(block_hash(2)).await.unwrap();

        assert_eq!(fixture.coins().await, [coin]);
    }

    #[tokio::test]
    async fn a_coin_whose_block_is_undone_is_no_longer_listed() {
        let fixture = Fixture::new().await;
        let (kept, undone) = (fixture.coin(1, 5_000).await, fixture.coin(2, 6_000).await);
        assert_eq!(fixture.coins().await, [kept, undone]);

        fixture.chain.undo_block(block_hash(2)).await.unwrap();

        assert_eq!(fixture.coins().await, [kept]);
    }

    #[tokio::test]
    async fn a_coin_with_a_recorded_spend_is_no_longer_listed() {
        let fixture = Fixture::new().await;
        let (first, second, other) = (
            fixture.coin(1, 5_000).await,
            fixture.coin(2, 6_000).await,
            fixture.coin(3, 7_000).await,
        );

        fixture.record_spends(txid(8), &[]).await;
        assert_eq!(fixture.coins().await, [first, second, other]);

        fixture.record_spends(txid(8), &[first, second]).await;
        assert_eq!(fixture.coins().await, [other]);
    }

    #[tokio::test]
    async fn a_coin_whose_spend_was_recorded_before_its_block_is_never_listed() {
        let fixture = Fixture::new().await;
        let address = fixture.addresses(1).await.remove(0);
        let coin = output(1);

        fixture.record_spends(txid(8), &[coin]).await;
        fixture.add_block(1, &[(coin, &address, 5_000)], &[]).await;

        assert!(fixture.coins().await.is_empty());
    }

    #[tokio::test]
    async fn a_spend_recorded_again_is_recorded_once() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1, 5_000).await;

        fixture.record_spends(txid(8), &[coin, coin]).await;
        fixture.record_spends(txid(8), &[coin]).await;

        let recorded: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM brz_ssp_onchain_wallet_spends")
                .fetch_one(&*fixture.pool)
                .await
                .unwrap();
        assert_eq!(recorded, 1);
        assert!(fixture.coins().await.is_empty());
    }

    #[tokio::test]
    async fn the_coins_of_forgotten_transactions_are_listed_again() {
        let fixture = Fixture::new().await;
        let (first, second, other) = (
            fixture.coin(1, 5_000).await,
            fixture.coin(2, 6_000).await,
            fixture.coin(3, 7_000).await,
        );
        fixture.record_spends(txid(7), &[first]).await;
        fixture.record_spends(txid(8), &[second]).await;
        fixture.record_spends(txid(9), &[other]).await;

        fixture.forget_spends(&[txid(7), txid(8), txid(8)]).await;

        assert_eq!(fixture.coins().await, [first, second]);
    }

    #[tokio::test]
    async fn forgetting_no_transaction_or_one_without_a_recorded_spend_frees_no_coin() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1, 5_000).await;
        fixture.record_spends(txid(8), &[coin]).await;

        fixture.forget_spends(&[]).await;
        fixture.forget_spends(&[txid(9)]).await;

        assert!(fixture.coins().await.is_empty());
    }

    #[tokio::test]
    async fn a_coin_with_recorded_spends_of_two_transactions_is_listed_once_both_are_forgotten() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1, 5_000).await;
        fixture.record_spends(txid(8), &[coin]).await;
        fixture.record_spends(txid(9), &[coin]).await;

        fixture.forget_spends(&[txid(8)]).await;
        assert!(fixture.coins().await.is_empty());

        fixture.forget_spends(&[txid(9)]).await;
        assert_eq!(fixture.coins().await, [coin]);
    }

    #[tokio::test]
    async fn a_recorded_spend_frees_its_other_coins_once_a_conflicting_spend_has_six_confirmations()
    {
        let fixture = Fixture::new().await;
        let (lost, other) = (fixture.coin(1, 5_000).await, fixture.coin(2, 6_000).await);
        fixture.record_spends(txid(8), &[lost, other]).await;
        fixture.add_block(10, &[], &[(lost, txid(9))]).await;

        fixture.add_block(14, &[], &[]).await;
        assert!(fixture.coins().await.is_empty(), "five confirmations");

        fixture.add_block(15, &[], &[]).await;
        assert_eq!(fixture.coins().await, [other], "six confirmations");
    }

    #[tokio::test]
    async fn the_coins_selected_for_a_build_are_kept_out_until_the_build_drops_them() {
        let fixture = Fixture::new().await;
        let (large, small) = (fixture.coin(1, 5_000).await, fixture.coin(2, 1_000).await);

        let (selected, held) = fixture.wallet.select_coins(&|_| 4_000).await.unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].outpoint, large);
        assert_eq!(fixture.coins().await, [small]);

        drop(held);
        assert_eq!(fixture.coins().await, [large, small]);
    }

    #[tokio::test]
    async fn the_largest_coins_are_selected_first_until_they_cover_the_need() {
        let fixture = Fixture::new().await;
        fixture.coin(1, 1_000).await;
        let (largest, second) = (fixture.coin(2, 5_000).await, fixture.coin(3, 3_000).await);

        let (selected, _held) = fixture.wallet.select_coins(&|_| 6_000).await.unwrap();

        let selected: Vec<OutPoint> = selected.iter().map(|coin| coin.outpoint).collect();
        assert_eq!(selected, [largest, second]);
    }

    #[tokio::test]
    async fn a_selection_fails_when_the_coins_do_not_cover_the_need() {
        let fixture = Fixture::new().await;
        let coins = [fixture.coin(1, 1_000).await, fixture.coin(2, 2_000).await];

        let selection = fixture.wallet.select_coins(&|_| 5_000).await;

        assert!(matches!(
            selection,
            Err(OnchainWalletError::InsufficientFunds {
                need: 5_000,
                have: 3_000
            })
        ));
        assert_eq!(fixture.coins().await, coins);
    }

    #[tokio::test]
    async fn a_selection_of_at_most_some_coins_fails_when_those_do_not_cover_the_need() {
        let fixture = Fixture::new().await;
        fixture.coin(1, 5_000).await;
        fixture.coin(2, 3_000).await;
        let wallet = &fixture.wallet;

        let one = wallet.select_at_most(1, &|_| 6_000).await;
        assert!(matches!(
            one,
            Err(OnchainWalletError::InsufficientFunds {
                need: 6_000,
                have: 5_000
            })
        ));

        let (two, _held) = wallet.select_at_most(2, &|_| 6_000).await.unwrap();
        assert_eq!(two.len(), 2);
    }

    #[tokio::test]
    async fn the_wallets_outputs_at_the_given_outpoints_are_read_spent_or_not() {
        let fixture = Fixture::new().await;
        let addresses = fixture.addresses(3).await;
        let (unspent, spent, withheld) = (output(1), output(2), output(3));
        fixture
            .add_block(
                1,
                &[
                    (unspent, &addresses[0], 5_000),
                    (spent, &addresses[1], 6_000),
                    (withheld, &addresses[2], 7_000),
                ],
                &[],
            )
            .await;
        fixture.add_block(2, &[], &[(spent, txid(9))]).await;
        fixture.record_spends(txid(8), &[withheld]).await;
        let wallet = &fixture.wallet;

        assert!(wallet.outputs_at(&[]).await.unwrap().is_empty());
        let one = wallet.outputs_at(&[spent]).await.unwrap();
        assert_eq!(in_order(one), [(spent, 6_000, addresses[1].clone())]);
        let several = wallet.outputs_at(&[unspent, spent, withheld]).await;
        assert_eq!(
            in_order(several.unwrap()),
            [
                (unspent, 5_000, addresses[0].clone()),
                (spent, 6_000, addresses[1].clone()),
                (withheld, 7_000, addresses[2].clone()),
            ]
        );
    }

    #[tokio::test]
    async fn an_outpoint_that_is_no_output_of_the_wallets_is_left_out() {
        let fixture = Fixture::new().await;
        let own = fixture.coin(1, 5_000).await;
        let watched = taproot_address(&key(9));
        fixture
            .wallet
            .register_watch_addresses(std::slice::from_ref(&watched))
            .await
            .unwrap();
        let (at_a_watched_address, not_stored) = (output(2), output(3));
        fixture
            .add_block(2, &[(at_a_watched_address, &watched, 6_000)], &[])
            .await;

        let outputs = fixture
            .wallet
            .outputs_at(&[own, at_a_watched_address, not_stored])
            .await
            .unwrap();

        let outputs: Vec<OutPoint> = outputs.iter().map(|output| output.outpoint).collect();
        assert_eq!(outputs, [own]);
    }

    #[tokio::test]
    async fn the_wallets_output_is_returned_once_for_each_time_its_outpoint_is_given() {
        let fixture = Fixture::new().await;
        let coin = fixture.coin(1, 5_000).await;

        let outputs = fixture.wallet.outputs_at(&[coin, coin]).await.unwrap();

        let outputs: Vec<OutPoint> = outputs.iter().map(|output| output.outpoint).collect();
        assert_eq!(outputs, [coin, coin]);
    }
}
