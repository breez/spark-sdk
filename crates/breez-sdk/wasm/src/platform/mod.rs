#[cfg(feature = "browser")]
mod browser;
#[cfg(feature = "browser")]
pub(crate) use browser::indexed_db_storage::default_storage;

#[cfg(feature = "node-js")]
mod node_js;
#[cfg(feature = "node-js")]
pub(crate) use node_js::sqlite_storage::default_storage;

#[cfg(all(not(feature = "browser"), not(feature = "node-js")))]
pub(crate) fn default_storage(
    _data_dir: &str,
) -> crate::error::WasmResult<crate::persist::Storage> {
    use breez_sdk_core::SdkError;

    Err(SdkError::GenericError("No storage implementation available".to_string()).into())
}
