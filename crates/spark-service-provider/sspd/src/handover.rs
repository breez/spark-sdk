use bitcoin::secp256k1::PublicKey;
use spark::operator::rpc::OperatorRpcError;
use spark::operator::rpc::spark::{TransferFilter, transfer_filter::Participant};
use spark::operator::{Operator, OperatorPool};
use spark::services::{ServiceError, Transfer, TransferId, TransferStatus};
use spark::tree::{LeavesReservation, LeavesReservationId, TreeNode, TreeNodeId, TreeStore};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoverReservation {
    pub id: LeavesReservationId,
    pub leaf_ids: Vec<TreeNodeId>,
}

impl HandoverReservation {
    pub async fn leaves(&self, tree_store: &dyn TreeStore) -> Result<LeavesReservation, BoxError> {
        let leaves: Vec<TreeNode> = tree_store
            .get_leaves()
            .await?
            .reserved_for_payment
            .into_iter()
            .filter(|leaf| self.leaf_ids.contains(&leaf.id))
            .collect();
        if leaves.len() != self.leaf_ids.len() {
            return Err(format!(
                "the leaf store holds {} of the {} leaves of reservation {}",
                leaves.len(),
                self.leaf_ids.len(),
                self.id
            )
            .into());
        }
        Ok(LeavesReservation::new(leaves, self.id.clone()))
    }
}

impl From<&LeavesReservation> for HandoverReservation {
    fn from(reservation: &LeavesReservation) -> Self {
        Self {
            id: reservation.id.clone(),
            leaf_ids: reservation
                .leaves
                .iter()
                .map(|leaf| leaf.id.clone())
                .collect(),
        }
    }
}

#[derive(Debug)]
pub enum HandoverOutcome {
    /// The coordinator holds a copy, which it only persists together with its
    /// decision to commit the call.
    Committed,
    /// An operator returned its copy before any expiry. The SSP never cancels a
    /// transfer, so the call was rolled back. `settled` once every copy is
    /// returned, which unlocks the leaves on every operator.
    RolledBack { settled: bool },
    /// `held` when an operator has a copy. Without one, making the call again
    /// under the same transfer id cannot hand the leaves over twice: an operator
    /// accepts a transfer id once.
    Undetermined { held: bool },
}

pub async fn observe_handover(
    operator_pool: &OperatorPool,
    sender: &PublicKey,
    network: spark::Network,
    transfer_id: &TransferId,
) -> Result<HandoverOutcome, ServiceError> {
    let coordinator = operator_pool.get_coordinator();
    if query_copy(coordinator, sender, network, transfer_id)
        .await?
        .is_some()
    {
        return Ok(HandoverOutcome::Committed);
    }
    let mut copies = Vec::new();
    for operator in operator_pool.get_non_coordinator_operators() {
        copies.extend(query_copy(operator, sender, network, transfer_id).await?);
    }
    Ok(outcome_of_copies(&copies))
}

/// The outcome the participants' copies show when the coordinator has none.
fn outcome_of_copies(copies: &[Transfer]) -> HandoverOutcome {
    if copies.iter().any(returned_by_rollback) {
        return HandoverOutcome::RolledBack {
            settled: copies
                .iter()
                .all(|copy| copy.status == TransferStatus::Returned),
        };
    }
    HandoverOutcome::Undetermined {
        held: !copies.is_empty(),
    }
}

/// Whether `copy` was returned other than for its expiry: an operator returns an
/// expired transfer only after its expiry, setting the transfer's update time.
/// A transfer without an expiry is reported as expiring at the Unix epoch.
fn returned_by_rollback(copy: &Transfer) -> bool {
    copy.status == TransferStatus::Returned
        && match copy.expiry_time {
            None | Some(0) => true,
            Some(expiry) => copy.updated_time.is_some_and(|updated| updated < expiry),
        }
}

/// Whether the operators turned a call down on its merits, rather than failing to
/// answer it.
pub fn is_refusal(error: &OperatorRpcError) -> bool {
    matches!(
        error,
        OperatorRpcError::Connection(status)
            if matches!(
                status.code(),
                tonic::Code::InvalidArgument | tonic::Code::FailedPrecondition
            )
    )
}

async fn query_copy(
    operator: &Operator,
    sender: &PublicKey,
    network: spark::Network,
    transfer_id: &TransferId,
) -> Result<Option<Transfer>, ServiceError> {
    let response = operator
        .client
        .query_all_transfers(TransferFilter {
            transfer_ids: vec![transfer_id.to_string()],
            participant: Some(Participant::SenderIdentityPublicKey(
                sender.serialize().to_vec(),
            )),
            network: network.to_proto_network() as i32,
            ..Default::default()
        })
        .await?;
    response
        .transfers
        .into_iter()
        .next()
        .map(Transfer::try_from)
        .transpose()
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use spark::services::{Transfer, TransferId, TransferStatus, TransferType};

    use super::{HandoverOutcome, outcome_of_copies};

    fn copy(status: TransferStatus, expiry_time: Option<u64>, updated_time: u64) -> Transfer {
        let key = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[1; 32]).unwrap(),
        );
        Transfer {
            id: TransferId::generate(),
            sender_identity_public_key: key,
            receiver_identity_public_key: key,
            status,
            total_value: 1_000,
            expiry_time,
            leaves: Vec::new(),
            created_time: Some(100),
            updated_time: Some(updated_time),
            transfer_type: TransferType::PreimageSwap,
            spark_invoice: None,
        }
    }

    #[test]
    fn no_copy_anywhere_is_undetermined_and_unheld() {
        assert!(matches!(
            outcome_of_copies(&[]),
            HandoverOutcome::Undetermined { held: false }
        ));
    }

    #[test]
    fn a_prepared_copy_is_held_until_its_call_ends() {
        assert!(matches!(
            outcome_of_copies(&[copy(TransferStatus::SenderKeyTweakPending, Some(0), 150)]),
            HandoverOutcome::Undetermined { held: true }
        ));
    }

    #[test]
    fn a_copy_returned_without_an_expiry_was_rolled_back() {
        let returned = copy(TransferStatus::Returned, Some(0), 150);
        let prepared = copy(TransferStatus::SenderKeyTweakPending, Some(0), 150);
        assert!(matches!(
            outcome_of_copies(&[returned.clone(), prepared]),
            HandoverOutcome::RolledBack { settled: false }
        ));
        assert!(matches!(
            outcome_of_copies(&[returned.clone(), returned]),
            HandoverOutcome::RolledBack { settled: true }
        ));
    }

    #[test]
    fn a_copy_returned_once_its_expiry_passed_proves_nothing() {
        assert!(matches!(
            outcome_of_copies(&[copy(TransferStatus::Returned, Some(200), 250)]),
            HandoverOutcome::Undetermined { held: true }
        ));
        assert!(matches!(
            outcome_of_copies(&[copy(TransferStatus::Returned, Some(200), 150)]),
            HandoverOutcome::RolledBack { settled: true }
        ));
    }
}
