use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Matched untagged, in order. `Error` comes before `Response` because Bitcoin
/// Core sets both `result` and `error` on every reply, so an error reply would
/// otherwise match `Response` with a null result.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RpcReply {
    Error { error: RpcError },
    Response { result: Value },
}

#[derive(Debug, Serialize)]
pub struct RpcRequest<TParams> {
    pub id: &'static str,
    pub jsonrpc: &'static str,
    pub method: String,
    pub params: TParams,
}

#[derive(Debug, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use crate::bitcoind::jsonrpc::RpcReply;

    #[test]
    fn test_deserialize_response() {
        let json = r#"{"id":"test","result":{}}"#;
        let reply = serde_json::from_str::<RpcReply>(json).unwrap();
        assert!(matches!(reply, RpcReply::Response { .. }));
    }

    #[test]
    fn test_deserialize_error() {
        let json = r#"{"id":"test","error":{"code":1,"message":"test","data":{}}}"#;
        let reply = serde_json::from_str::<RpcReply>(json).unwrap();
        assert!(matches!(reply, RpcReply::Error { .. }));
    }

    #[test]
    fn test_deserialize_response_with_null_error() {
        let json = r#"{"id":"test","result":"abc","error":null}"#;
        let reply = serde_json::from_str::<RpcReply>(json).unwrap();
        assert!(matches!(reply, RpcReply::Response { .. }));
    }

    #[test]
    fn test_deserialize_error_with_null_result() {
        let json = r#"{"id":"test","result":null,"error":{"code":-26,"message":"txn-already-in-mempool"}}"#;
        let reply = serde_json::from_str::<RpcReply>(json).unwrap();
        let RpcReply::Error { error } = reply else {
            panic!("expected an error body");
        };
        assert_eq!(error.code, -26);
        assert_eq!(error.message, "txn-already-in-mempool");
    }
}
