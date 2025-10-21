use std::{collections::HashMap, option::Option, string::String};

use breez_sdk_common::error::ServiceConnectivityError;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{
    JsFuture,
    js_sys::{self, Promise},
};

use crate::models::error::js_error_to_service_connectivity_error;

#[macros::extern_wasm_bindgen(breez_sdk_common::rest::RestResponse)]
pub struct RestResponse {
    pub status: u16,
    pub body: String,
}

pub struct WasmRestClient {
    pub inner: RestClient,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmRestClient {}
unsafe impl Sync for WasmRestClient {}

#[macros::async_trait]
impl breez_sdk_common::rest::RestClient for WasmRestClient {
    async fn get_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
    ) -> Result<breez_sdk_common::rest::RestResponse, ServiceConnectivityError> {
        let promise = self
            .inner
            .get_request(url, headers_to_js_value(headers)?)
            .map_err(js_error_to_service_connectivity_error)?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(js_error_to_service_connectivity_error)?;
        let rest_response: RestResponse = serde_wasm_bindgen::from_value(result)
            .map_err(|e| ServiceConnectivityError::Other(e.to_string()))?;
        Ok(rest_response.into())
    }

    async fn post_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<breez_sdk_common::rest::RestResponse, ServiceConnectivityError> {
        let promise = self
            .inner
            .post_request(url, headers_to_js_value(headers)?, body)
            .map_err(js_error_to_service_connectivity_error)?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(js_error_to_service_connectivity_error)?;
        let rest_response: RestResponse = serde_wasm_bindgen::from_value(result)
            .map_err(|e| ServiceConnectivityError::Other(e.to_string()))?;
        Ok(rest_response.into())
    }

    async fn delete_request(
        &self,
        url: String,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<breez_sdk_common::rest::RestResponse, ServiceConnectivityError> {
        let promise = self
            .inner
            .delete_request(url, headers_to_js_value(headers)?, body)
            .map_err(js_error_to_service_connectivity_error)?;
        let future = JsFuture::from(promise);
        let result = future
            .await
            .map_err(js_error_to_service_connectivity_error)?;
        let rest_response: RestResponse = serde_wasm_bindgen::from_value(result)
            .map_err(|e| ServiceConnectivityError::Other(e.to_string()))?;
        Ok(rest_response.into())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const EVENT_INTERFACE: &'static str = r#"export interface RestClient {
    getRequest(url: string, headers?: Record<string, string>): Promise<RestResponse>;
    postRequest(url: string, headers?: Record<string, string>, body?: string): Promise<RestResponse>;
    deleteRequest(url: string, headers?: Record<string, string>, body?: string): Promise<RestResponse>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "RestClient")]
    pub type RestClient;

    #[wasm_bindgen(structural, method, js_name = "getRequest", catch)]
    pub fn get_request(
        this: &RestClient,
        url: String,
        headers: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "postRequest", catch)]
    pub fn post_request(
        this: &RestClient,
        url: String,
        headers: JsValue,
        body: Option<String>,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = "deleteRequest", catch)]
    pub fn delete_request(
        this: &RestClient,
        url: String,
        headers: JsValue,
        body: Option<String>,
    ) -> Result<Promise, JsValue>;
}

fn headers_to_js_value(
    headers: Option<HashMap<String, String>>,
) -> Result<JsValue, ServiceConnectivityError> {
    match headers {
        Some(map) => {
            let js_obj = js_sys::Object::new();
            for (key, value) in map.iter() {
                js_sys::Reflect::set(&js_obj, &JsValue::from_str(key), &JsValue::from_str(value))
                    .map_err(js_error_to_service_connectivity_error)?;
            }
            Ok(js_obj.into())
        }
        None => Ok(JsValue::NULL),
    }
}
