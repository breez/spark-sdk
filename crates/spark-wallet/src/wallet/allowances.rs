use bitcoin::secp256k1::schnorr;
use spark::token::{
    NewTokenAllowance, PreparedTokenPull, PullReceiver, TokenAllowance, TokenAllowanceQuery,
};

use super::*;

impl SparkWallet {
    pub async fn create_token_allowance(
        &self,
        request: NewTokenAllowance,
    ) -> Result<TokenAllowance, SparkWalletError> {
        Ok(self
            .token_allowance_service
            .create_token_allowance(request)
            .await?)
    }

    pub async fn revoke_token_allowance(&self, allowance_id: &str) -> Result<(), SparkWalletError> {
        Ok(self
            .token_allowance_service
            .revoke_token_allowance(allowance_id)
            .await?)
    }

    pub async fn query_token_allowances(
        &self,
        query: TokenAllowanceQuery,
    ) -> Result<Vec<TokenAllowance>, SparkWalletError> {
        Ok(self
            .token_allowance_service
            .query_token_allowances(query)
            .await?)
    }

    pub async fn prepare_token_pull(
        &self,
        payer: PublicKey,
        token_identifier: &str,
        receivers: Vec<PullReceiver>,
    ) -> Result<PreparedTokenPull, SparkWalletError> {
        Ok(self
            .token_allowance_service
            .prepare_token_pull(payer, token_identifier, receivers)
            .await?)
    }

    pub async fn sign_and_broadcast_token_pull(
        &self,
        prepared: &PreparedTokenPull,
    ) -> Result<TokenTransaction, SparkWalletError> {
        Ok(self
            .token_allowance_service
            .sign_and_broadcast_token_pull(prepared)
            .await?)
    }

    pub async fn broadcast_token_pull(
        &self,
        prepared: &PreparedTokenPull,
        signature: schnorr::Signature,
    ) -> Result<TokenTransaction, SparkWalletError> {
        Ok(self
            .token_allowance_service
            .broadcast_token_pull(prepared, &signature)
            .await?)
    }
}
