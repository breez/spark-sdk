use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::operator::rpc::OperatorRpcError;
use crate::services::ServiceError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenAllowanceFailure {
    NotEnabled,
    NotFound,
    Revoked,
    Expired,
    NotSpendable,
    OverPerPaymentLimit,
    OverTotalLimit,
    RecipientNotAllowed,
    PayerInsufficientFunds,
    AlreadyActive,
    QuotaExceeded,
    PreparedPullStale,
}

impl TokenAllowanceFailure {
    pub(crate) fn into_error(self, message: impl Into<String>) -> ServiceError {
        ServiceError::TokenAllowance {
            failure: self,
            message: message.into(),
        }
    }
}

const OPERATOR_MESSAGES: [(&str, TokenAllowanceFailure); 9] = [
    ("token allowance not found", TokenAllowanceFailure::NotFound),
    (
        "token allowance has been revoked",
        TokenAllowanceFailure::Revoked,
    ),
    (
        "token allowance has expired",
        TokenAllowanceFailure::Expired,
    ),
    (
        "metered amount exceeds the allowance per-transaction cap",
        TokenAllowanceFailure::OverPerPaymentLimit,
    ),
    (
        "metered amount exceeds the allowance remaining budget",
        TokenAllowanceFailure::OverTotalLimit,
    ),
    (
        "output recipient is not in the allowance recipient allowlist",
        TokenAllowanceFailure::RecipientNotAllowed,
    ),
    (
        "token allowance is not in a spendable state",
        TokenAllowanceFailure::NotSpendable,
    ),
    (
        "an active allowance already exists for this owner, spender, and token",
        TokenAllowanceFailure::AlreadyActive,
    ),
    ("the per-owner cap is", TokenAllowanceFailure::QuotaExceeded),
];

fn operator_status(error: &ServiceError) -> Option<&Status> {
    match error {
        ServiceError::ServiceConnectionError(rpc_error) => match rpc_error.as_ref() {
            OperatorRpcError::Connection(status) => Some(status),
            _ => None,
        },
        ServiceError::RequestError(status) => Some(status),
        _ => None,
    }
}

fn allowance_failure(status: &Status) -> Option<TokenAllowanceFailure> {
    if status.code() == Code::Unimplemented {
        return Some(TokenAllowanceFailure::NotEnabled);
    }
    OPERATOR_MESSAGES
        .iter()
        .find(|(needle, _)| status.message().contains(needle))
        .map(|(_, failure)| *failure)
}

pub(crate) fn classify_allowance_error(error: ServiceError) -> ServiceError {
    let classified = operator_status(&error).and_then(|status| {
        allowance_failure(status).map(|failure| (failure, status.message().to_string()))
    });
    match classified {
        Some((failure, message)) => failure.into_error(message),
        None => error,
    }
}

const TRANSACTION_PREEMPTED: &str = "TRANSACTION_PREEMPTED";
const OUT_OF_RANGE: &str = "OUT_OF_RANGE";

fn pull_never_accepted(status: &Status, window_passed: bool) -> bool {
    let reason = status.get_details_error_info().map(|info| info.reason);
    match (status.code(), reason.as_deref()) {
        (Code::Aborted, Some(TRANSACTION_PREEMPTED)) => true,
        (Code::InvalidArgument, Some(OUT_OF_RANGE)) => window_passed,
        _ => false,
    }
}

pub(crate) fn classify_pull_error(error: ServiceError, window_passed: bool) -> ServiceError {
    let stale = operator_status(&error)
        .filter(|status| pull_never_accepted(status, window_passed))
        .map(|status| status.message().to_string());
    match stale {
        Some(message) => TokenAllowanceFailure::PreparedPullStale.into_error(message),
        None => classify_allowance_error(error),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use macros::test_all;
    use tonic::{Code, Status};
    use tonic_types::{ErrorDetails, StatusExt};

    use super::{TokenAllowanceFailure, classify_allowance_error, classify_pull_error};
    use crate::{operator::rpc::OperatorRpcError, services::ServiceError};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn rpc_error(code: Code, message: &str) -> ServiceError {
        ServiceError::ServiceConnectionError(Box::new(OperatorRpcError::Connection(Box::new(
            Status::new(code, message),
        ))))
    }

    fn error_with_reason(code: Code, reason: &str) -> ServiceError {
        let details = ErrorDetails::with_error_info(reason, "spark", HashMap::new());
        ServiceError::ServiceConnectionError(Box::new(OperatorRpcError::Connection(Box::new(
            Status::with_error_details(code, "operator rejected", details),
        ))))
    }

    fn failure(error: ServiceError) -> Option<TokenAllowanceFailure> {
        match error {
            ServiceError::TokenAllowance { failure, .. } => Some(failure),
            _ => None,
        }
    }

    #[test_all]
    fn classifies_operator_refusals() {
        let cases = [
            (
                Code::Unimplemented,
                "token allowances are not enabled",
                TokenAllowanceFailure::NotEnabled,
            ),
            (
                Code::NotFound,
                "token allowance not found",
                TokenAllowanceFailure::NotFound,
            ),
            (
                Code::FailedPrecondition,
                "sign phase failed: token allowance has been revoked",
                TokenAllowanceFailure::Revoked,
            ),
            (
                Code::FailedPrecondition,
                "failed to meter allowance spend transaction: metered amount exceeds the allowance per-transaction cap",
                TokenAllowanceFailure::OverPerPaymentLimit,
            ),
            (
                Code::FailedPrecondition,
                "metered amount exceeds the allowance remaining budget",
                TokenAllowanceFailure::OverTotalLimit,
            ),
            (
                Code::FailedPrecondition,
                "output recipient is not in the allowance recipient allowlist",
                TokenAllowanceFailure::RecipientNotAllowed,
            ),
            (
                Code::AlreadyExists,
                "an active allowance already exists for this owner, spender, and token",
                TokenAllowanceFailure::AlreadyActive,
            ),
            (
                Code::ResourceExhausted,
                "the per-owner cap is 100",
                TokenAllowanceFailure::QuotaExceeded,
            ),
        ];
        for (code, message, expected) in cases {
            assert_eq!(
                failure(classify_allowance_error(rpc_error(code, message))),
                Some(expected)
            );
        }
    }

    #[test_all]
    fn leaves_unrelated_errors_alone() {
        let unrelated = [
            (Code::Unavailable, "connection reset"),
            (
                Code::Aborted,
                "inputs cannot be spent: token transaction with these inputs is already in progress or finalized",
            ),
            (
                Code::FailedPrecondition,
                "failed to commit transaction: at least one input is frozen. Cannot proceed with transaction",
            ),
        ];
        for (code, message) in unrelated {
            assert!(matches!(
                classify_allowance_error(rpc_error(code, message)),
                ServiceError::ServiceConnectionError(_)
            ));
        }
    }

    #[test_all]
    fn a_pull_the_operators_never_accepted_is_stale() {
        let stale = [
            (Code::Aborted, "TRANSACTION_PREEMPTED", false),
            (Code::InvalidArgument, "OUT_OF_RANGE", true),
        ];
        for (code, reason, window_passed) in stale {
            assert_eq!(
                failure(classify_pull_error(
                    error_with_reason(code, reason),
                    window_passed
                )),
                Some(TokenAllowanceFailure::PreparedPullStale)
            );
        }
    }

    #[test_all]
    fn other_pull_errors_are_not_stale() {
        let others = [
            (
                error_with_reason(Code::InvalidArgument, "OUT_OF_RANGE"),
                false,
            ),
            (error_with_reason(Code::Aborted, "LOCK_CONFLICT"), true),
            (
                error_with_reason(Code::AlreadyExists, "EXPIRED_TRANSACTION"),
                true,
            ),
            (
                error_with_reason(Code::AlreadyExists, "DUPLICATE_OPERATION"),
                true,
            ),
            (rpc_error(Code::Aborted, "inputs cannot be spent"), true),
        ];
        for (error, window_passed) in others {
            assert!(matches!(
                classify_pull_error(error, window_passed),
                ServiceError::ServiceConnectionError(_)
            ));
        }
    }
}
