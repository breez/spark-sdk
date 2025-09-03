#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
// EXTRA BEGIN
typedef struct DartCObject *WireSyncRust2DartDco;
typedef struct WireSyncRust2DartSse {
  uint8_t *ptr;
  int32_t len;
} WireSyncRust2DartSse;

typedef int64_t DartPort;
typedef bool (*DartPostCObjectFnType)(DartPort port_id, void *message);
void store_dart_post_cobject(DartPostCObjectFnType ptr);
// EXTRA END
typedef struct _Dart_Handle* Dart_Handle;

typedef struct wire_cst_list_prim_u_8_strict {
  uint8_t *ptr;
  int32_t len;
} wire_cst_list_prim_u_8_strict;

typedef struct wire_cst_claim_deposit_request {
  struct wire_cst_list_prim_u_8_strict *txid;
  uint32_t vout;
  uintptr_t *max_fee;
} wire_cst_claim_deposit_request;

typedef struct wire_cst_get_info_request {

} wire_cst_get_info_request;

typedef struct wire_cst_get_payment_request {
  struct wire_cst_list_prim_u_8_strict *payment_id;
} wire_cst_get_payment_request;

typedef struct wire_cst_list_payments_request {
  uint32_t *offset;
  uint32_t *limit;
} wire_cst_list_payments_request;

typedef struct wire_cst_list_unclaimed_deposits_request {

} wire_cst_list_unclaimed_deposits_request;

typedef struct wire_cst_lnurl_pay_request_details {
  struct wire_cst_list_prim_u_8_strict *callback;
  uint64_t min_sendable;
  uint64_t max_sendable;
  struct wire_cst_list_prim_u_8_strict *metadata_str;
  uint16_t comment_allowed;
  struct wire_cst_list_prim_u_8_strict *domain;
  struct wire_cst_list_prim_u_8_strict *url;
  struct wire_cst_list_prim_u_8_strict *address;
  bool allows_nostr;
  struct wire_cst_list_prim_u_8_strict *nostr_pubkey;
} wire_cst_lnurl_pay_request_details;

typedef struct wire_cst_payment_request_source {
  struct wire_cst_list_prim_u_8_strict *bip_21_uri;
  struct wire_cst_list_prim_u_8_strict *bip_353_address;
} wire_cst_payment_request_source;

typedef struct wire_cst_bolt_11_invoice {
  struct wire_cst_list_prim_u_8_strict *bolt11;
  struct wire_cst_payment_request_source source;
} wire_cst_bolt_11_invoice;

typedef struct wire_cst_bolt_11_route_hint_hop {
  struct wire_cst_list_prim_u_8_strict *src_node_id;
  struct wire_cst_list_prim_u_8_strict *short_channel_id;
  uint32_t fees_base_msat;
  uint32_t fees_proportional_millionths;
  uint16_t cltv_expiry_delta;
  uint64_t *htlc_minimum_msat;
  uint64_t *htlc_maximum_msat;
} wire_cst_bolt_11_route_hint_hop;

typedef struct wire_cst_list_bolt_11_route_hint_hop {
  struct wire_cst_bolt_11_route_hint_hop *ptr;
  int32_t len;
} wire_cst_list_bolt_11_route_hint_hop;

typedef struct wire_cst_bolt_11_route_hint {
  struct wire_cst_list_bolt_11_route_hint_hop *hops;
} wire_cst_bolt_11_route_hint;

typedef struct wire_cst_list_bolt_11_route_hint {
  struct wire_cst_bolt_11_route_hint *ptr;
  int32_t len;
} wire_cst_list_bolt_11_route_hint;

typedef struct wire_cst_bolt_11_invoice_details {
  uint64_t *amount_msat;
  struct wire_cst_list_prim_u_8_strict *description;
  struct wire_cst_list_prim_u_8_strict *description_hash;
  uint64_t expiry;
  struct wire_cst_bolt_11_invoice invoice;
  uint64_t min_final_cltv_expiry_delta;
  int32_t network;
  struct wire_cst_list_prim_u_8_strict *payee_pubkey;
  struct wire_cst_list_prim_u_8_strict *payment_hash;
  struct wire_cst_list_prim_u_8_strict *payment_secret;
  struct wire_cst_list_bolt_11_route_hint *routing_hints;
  uint64_t timestamp;
} wire_cst_bolt_11_invoice_details;

typedef struct wire_cst_prepare_lnurl_pay_response {
  uint64_t amount_sats;
  struct wire_cst_list_prim_u_8_strict *comment;
  struct wire_cst_lnurl_pay_request_details pay_request;
  uint64_t fee_sats;
  struct wire_cst_bolt_11_invoice_details invoice_details;
  uintptr_t *success_action;
} wire_cst_prepare_lnurl_pay_response;

typedef struct wire_cst_lnurl_pay_request {
  struct wire_cst_prepare_lnurl_pay_response prepare_response;
} wire_cst_lnurl_pay_request;

typedef struct wire_cst_prepare_lnurl_pay_request {
  uint64_t amount_sats;
  struct wire_cst_lnurl_pay_request_details pay_request;
  struct wire_cst_list_prim_u_8_strict *comment;
  bool *validate_success_action_url;
} wire_cst_prepare_lnurl_pay_request;

typedef struct wire_cst_prepare_send_payment_request {
  struct wire_cst_list_prim_u_8_strict *payment_request;
  uint64_t *amount_sats;
} wire_cst_prepare_send_payment_request;

typedef struct wire_cst_sync_wallet_request {

} wire_cst_sync_wallet_request;

typedef struct wire_cst_EventListenerImplementor_Variant0 {
  uintptr_t field0;
} wire_cst_EventListenerImplementor_Variant0;

typedef union EventListenerImplementorKind {
  struct wire_cst_EventListenerImplementor_Variant0 Variant0;
} EventListenerImplementorKind;

typedef struct wire_cst_event_listener_implementor {
  int32_t tag;
  union EventListenerImplementorKind kind;
} wire_cst_event_listener_implementor;

typedef struct wire_cst_record_string_string {
  struct wire_cst_list_prim_u_8_strict *field0;
  struct wire_cst_list_prim_u_8_strict *field1;
} wire_cst_record_string_string;

typedef struct wire_cst_list_record_string_string {
  struct wire_cst_record_string_string *ptr;
  int32_t len;
} wire_cst_list_record_string_string;

typedef struct wire_cst_RestClientImplementor_Variant0 {
  uintptr_t field0;
} wire_cst_RestClientImplementor_Variant0;

typedef union RestClientImplementorKind {
  struct wire_cst_RestClientImplementor_Variant0 Variant0;
} RestClientImplementorKind;

typedef struct wire_cst_rest_client_implementor {
  int32_t tag;
  union RestClientImplementorKind kind;
} wire_cst_rest_client_implementor;

typedef struct wire_cst_config {
  struct wire_cst_list_prim_u_8_strict *api_key;
  int32_t network;
  uint32_t sync_interval_secs;
  uintptr_t *max_deposit_claim_fee;
} wire_cst_config;

typedef struct wire_cst_payment {
  struct wire_cst_list_prim_u_8_strict *id;
  int32_t payment_type;
  int32_t status;
  uint64_t amount;
  uint64_t fees;
  uint64_t timestamp;
  int32_t method;
  uintptr_t *details;
} wire_cst_payment;

typedef struct wire_cst_lnurl_pay_info {
  struct wire_cst_list_prim_u_8_strict *ln_address;
  struct wire_cst_list_prim_u_8_strict *comment;
  struct wire_cst_list_prim_u_8_strict *domain;
  struct wire_cst_list_prim_u_8_strict *metadata;
  uintptr_t *processed_success_action;
  uintptr_t *raw_success_action;
} wire_cst_lnurl_pay_info;

typedef struct wire_cst_payment_metadata {
  struct wire_cst_lnurl_pay_info *lnurl_pay_info;
} wire_cst_payment_metadata;

typedef struct wire_cst_binding_event_listener {
  struct wire_cst_list_prim_u_8_strict *listener;
} wire_cst_binding_event_listener;

typedef struct wire_cst_binding_logger {
  struct wire_cst_list_prim_u_8_strict *logger;
} wire_cst_binding_logger;

typedef struct wire_cst_log_entry {
  struct wire_cst_list_prim_u_8_strict *line;
  struct wire_cst_list_prim_u_8_strict *level;
} wire_cst_log_entry;

typedef struct wire_cst_connect_request {
  struct wire_cst_config config;
  struct wire_cst_list_prim_u_8_strict *mnemonic;
  struct wire_cst_list_prim_u_8_strict *storage_dir;
} wire_cst_connect_request;

typedef struct wire_cst_LoggerImplementor_Variant0 {
  uintptr_t field0;
} wire_cst_LoggerImplementor_Variant0;

typedef union LoggerImplementorKind {
  struct wire_cst_LoggerImplementor_Variant0 Variant0;
} LoggerImplementorKind;

typedef struct wire_cst_logger_implementor {
  int32_t tag;
  union LoggerImplementorKind kind;
} wire_cst_logger_implementor;

typedef struct wire_cst_send_onchain_speed_fee_quote {
  uint64_t user_fee_sat;
  uint64_t l1_broadcast_fee_sat;
} wire_cst_send_onchain_speed_fee_quote;

typedef struct wire_cst_list_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType {
  uintptr_t *ptr;
  int32_t len;
} wire_cst_list_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType;

typedef struct wire_cst_bip_21_extra {
  struct wire_cst_list_prim_u_8_strict *key;
  struct wire_cst_list_prim_u_8_strict *value;
} wire_cst_bip_21_extra;

typedef struct wire_cst_list_bip_21_extra {
  struct wire_cst_bip_21_extra *ptr;
  int32_t len;
} wire_cst_list_bip_21_extra;

typedef struct wire_cst_deposit_info {
  struct wire_cst_list_prim_u_8_strict *txid;
  uint32_t vout;
  uint64_t amount_sats;
  struct wire_cst_list_prim_u_8_strict *refund_tx;
  struct wire_cst_list_prim_u_8_strict *refund_tx_id;
  uintptr_t *claim_error;
} wire_cst_deposit_info;

typedef struct wire_cst_list_deposit_info {
  struct wire_cst_deposit_info *ptr;
  int32_t len;
} wire_cst_list_deposit_info;

typedef struct wire_cst_list_payment {
  struct wire_cst_payment *ptr;
  int32_t len;
} wire_cst_list_payment;

typedef struct wire_cst_bip_21_details {
  uint64_t *amount_sat;
  struct wire_cst_list_prim_u_8_strict *asset_id;
  struct wire_cst_list_prim_u_8_strict *uri;
  struct wire_cst_list_bip_21_extra *extras;
  struct wire_cst_list_prim_u_8_strict *label;
  struct wire_cst_list_prim_u_8_strict *message;
  struct wire_cst_list_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType *payment_methods;
} wire_cst_bip_21_details;

typedef struct wire_cst_claim_deposit_response {
  struct wire_cst_payment payment;
} wire_cst_claim_deposit_response;

typedef struct wire_cst_get_info_response {
  uint64_t balance_sats;
} wire_cst_get_info_response;

typedef struct wire_cst_get_payment_response {
  struct wire_cst_payment payment;
} wire_cst_get_payment_response;

typedef struct wire_cst_list_payments_response {
  struct wire_cst_list_payment *payments;
} wire_cst_list_payments_response;

typedef struct wire_cst_list_unclaimed_deposits_response {
  struct wire_cst_list_deposit_info *deposits;
} wire_cst_list_unclaimed_deposits_response;

typedef struct wire_cst_lnurl_pay_response {
  struct wire_cst_payment payment;
  uintptr_t *success_action;
} wire_cst_lnurl_pay_response;

typedef struct wire_cst_receive_payment_response {
  struct wire_cst_list_prim_u_8_strict *payment_request;
  uint64_t fee_sats;
} wire_cst_receive_payment_response;

typedef struct wire_cst_refund_deposit_response {
  struct wire_cst_list_prim_u_8_strict *tx_id;
  struct wire_cst_list_prim_u_8_strict *tx_hex;
} wire_cst_refund_deposit_response;

typedef struct wire_cst_send_payment_response {
  struct wire_cst_payment payment;
} wire_cst_send_payment_response;

typedef struct wire_cst_sync_wallet_response {

} wire_cst_sync_wallet_response;

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__BasicAuth_new(int64_t port_,
                                                                                     struct wire_cst_list_prim_u_8_strict *username,
                                                                                     struct wire_cst_list_prim_u_8_strict *password);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_claim_deposit(int64_t port_,
                                                                          uintptr_t that,
                                                                          struct wire_cst_claim_deposit_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_disconnect(int64_t port_,
                                                                       uintptr_t that);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_frb_override_add_event_listener(int64_t port_,
                                                                                            uintptr_t that,
                                                                                            struct wire_cst_list_prim_u_8_strict *listener);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_get_info(int64_t port_,
                                                                     uintptr_t that,
                                                                     struct wire_cst_get_info_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_get_payment(int64_t port_,
                                                                        uintptr_t that,
                                                                        struct wire_cst_get_payment_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_list_payments(int64_t port_,
                                                                          uintptr_t that,
                                                                          struct wire_cst_list_payments_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_list_unclaimed_deposits(int64_t port_,
                                                                                    uintptr_t that,
                                                                                    struct wire_cst_list_unclaimed_deposits_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_lnurl_pay(int64_t port_,
                                                                      uintptr_t that,
                                                                      struct wire_cst_lnurl_pay_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_prepare_lnurl_pay(int64_t port_,
                                                                              uintptr_t that,
                                                                              struct wire_cst_prepare_lnurl_pay_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_prepare_send_payment(int64_t port_,
                                                                                 uintptr_t that,
                                                                                 struct wire_cst_prepare_send_payment_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_receive_payment(int64_t port_,
                                                                            uintptr_t that,
                                                                            uintptr_t request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_refund_deposit(int64_t port_,
                                                                           uintptr_t that,
                                                                           uintptr_t request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_remove_event_listener(int64_t port_,
                                                                                  uintptr_t that,
                                                                                  struct wire_cst_list_prim_u_8_strict *id);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_send_payment(int64_t port_,
                                                                         uintptr_t that,
                                                                         uintptr_t request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_sync_wallet(int64_t port_,
                                                                        uintptr_t that,
                                                                        struct wire_cst_sync_wallet_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__breez_server__BreezServer_fetch_fiat_currencies(int64_t port_,
                                                                                                    uintptr_t that);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__breez_server__BreezServer_fetch_fiat_rates(int64_t port_,
                                                                                               uintptr_t that);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_add_listener(int64_t port_,
                                                                             uintptr_t that,
                                                                             struct wire_cst_event_listener_implementor *listener);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_default(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_emit(int64_t port_,
                                                                     uintptr_t that,
                                                                     uintptr_t event);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_new(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_remove_listener(int64_t port_,
                                                                                uintptr_t that,
                                                                                struct wire_cst_list_prim_u_8_strict *id);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__Fee_to_sats(int64_t port_,
                                                                       uintptr_t that,
                                                                       uint64_t vbytes);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_get_amount_sats(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_get_payment_method(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_set_amount_sats(uintptr_t that,
                                                                                                                                    uint64_t amount_sats);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_set_payment_method(uintptr_t that,
                                                                                                                                       uintptr_t payment_method);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__ReceivePaymentRequest_auto_accessor_get_payment_method(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__ReceivePaymentRequest_auto_accessor_set_payment_method(uintptr_t that,
                                                                                                                                  uintptr_t payment_method);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_destination_address(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_fee(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_txid(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_vout(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_destination_address(uintptr_t that,
                                                                                                                                      struct wire_cst_list_prim_u_8_strict *destination_address);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_fee(uintptr_t that,
                                                                                                                      uintptr_t fee);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_txid(uintptr_t that,
                                                                                                                       struct wire_cst_list_prim_u_8_strict *txid);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_vout(uintptr_t that,
                                                                                                                       uint32_t vout);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__rest__rest_client__ReqwestRestClient_get(int64_t port_,
                                                                                             uintptr_t that,
                                                                                             struct wire_cst_list_prim_u_8_strict *url,
                                                                                             struct wire_cst_list_record_string_string *headers);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__rest__rest_client__ReqwestRestClient_new(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__rest__rest_client__ReqwestRestClient_post(int64_t port_,
                                                                                              uintptr_t that,
                                                                                              struct wire_cst_list_prim_u_8_strict *url,
                                                                                              struct wire_cst_list_record_string_string *headers,
                                                                                              struct wire_cst_list_prim_u_8_strict *body);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_broadcast_transaction(int64_t port_,
                                                                                                                    uintptr_t that,
                                                                                                                    struct wire_cst_list_prim_u_8_strict *tx);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_get_address_utxos(int64_t port_,
                                                                                                                uintptr_t that,
                                                                                                                struct wire_cst_list_prim_u_8_strict *address);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_get_transaction_hex(int64_t port_,
                                                                                                                  uintptr_t that,
                                                                                                                  struct wire_cst_list_prim_u_8_strict *txid);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_new(int64_t port_,
                                                                                                  struct wire_cst_list_prim_u_8_strict *base_url,
                                                                                                  int32_t network,
                                                                                                  uintptr_t max_retries,
                                                                                                  struct wire_cst_rest_client_implementor *rest_client,
                                                                                                  uintptr_t *basic_auth);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SdkBuilder_build(int64_t port_, uintptr_t that);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SdkBuilder_new(int64_t port_,
                                                                  struct wire_cst_config *config,
                                                                  struct wire_cst_list_prim_u_8_strict *mnemonic,
                                                                  uintptr_t storage);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_get_options(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_get_prepare_response(uintptr_t that);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_set_options(uintptr_t that,
                                                                                                                        uintptr_t *options);

WireSyncRust2DartDco frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_set_prepare_response(uintptr_t that,
                                                                                                                                 uintptr_t prepare_response);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_add_deposit(int64_t port_,
                                                                             uintptr_t that,
                                                                             struct wire_cst_list_prim_u_8_strict *txid,
                                                                             uint32_t vout,
                                                                             uint64_t amount_sats);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_delete_deposit(int64_t port_,
                                                                                uintptr_t that,
                                                                                struct wire_cst_list_prim_u_8_strict *txid,
                                                                                uint32_t vout);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_get_cached_item(int64_t port_,
                                                                                 uintptr_t that,
                                                                                 struct wire_cst_list_prim_u_8_strict *key);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_get_payment_by_id(int64_t port_,
                                                                                   uintptr_t that,
                                                                                   struct wire_cst_list_prim_u_8_strict *id);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_insert_payment(int64_t port_,
                                                                                uintptr_t that,
                                                                                struct wire_cst_payment *payment);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_list_deposits(int64_t port_,
                                                                               uintptr_t that);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_list_payments(int64_t port_,
                                                                               uintptr_t that,
                                                                               uint32_t *offset,
                                                                               uint32_t *limit);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_new(int64_t port_, uintptr_t path);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_set_cached_item(int64_t port_,
                                                                                 uintptr_t that,
                                                                                 struct wire_cst_list_prim_u_8_strict *key,
                                                                                 struct wire_cst_list_prim_u_8_strict *value);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_set_payment_metadata(int64_t port_,
                                                                                      uintptr_t that,
                                                                                      struct wire_cst_list_prim_u_8_strict *payment_id,
                                                                                      struct wire_cst_payment_metadata *metadata);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_update_deposit(int64_t port_,
                                                                                uintptr_t that,
                                                                                struct wire_cst_list_prim_u_8_strict *txid,
                                                                                uint32_t vout,
                                                                                uintptr_t payload);

void frbgen_breez_sdk_spark_wire__crate__binding_event_listener_on_event(int64_t port_,
                                                                         struct wire_cst_binding_event_listener *that,
                                                                         uintptr_t e);

void frbgen_breez_sdk_spark_wire__crate__binding_logger_log(int64_t port_,
                                                            struct wire_cst_binding_logger *that,
                                                            struct wire_cst_log_entry *l);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bip_21_details_default(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bip_21_extra_default(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bolt_11_route_hint_default(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bolt_11_route_hint_hop_default(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__connect(int64_t port_,
                                                           struct wire_cst_connect_request *request);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__default_config(int64_t port_, int32_t network);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__default_storage(int64_t port_,
                                                                   struct wire_cst_list_prim_u_8_strict *data_dir);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__init_logging(int64_t port_,
                                                                struct wire_cst_list_prim_u_8_strict *log_dir,
                                                                struct wire_cst_logger_implementor *app_logger,
                                                                struct wire_cst_list_prim_u_8_strict *log_filter);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__lnurl_pay_info_default(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__input__parse(int64_t port_,
                                                                 struct wire_cst_list_prim_u_8_strict *input);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__parse(int64_t port_,
                                                         struct wire_cst_list_prim_u_8_strict *input);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__input__parse_invoice(int64_t port_,
                                                                         struct wire_cst_list_prim_u_8_strict *input);

void frbgen_breez_sdk_spark_wire__breez_sdk_common__input__payment_request_source_default(int64_t port_);

void frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__send_onchain_speed_fee_quote_total_fee_sat(int64_t port_,
                                                                                                      struct wire_cst_send_onchain_speed_fee_quote *that);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerArcdynStorage(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerArcdynStorage(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBasicAuth(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBasicAuth(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingEventListener(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingEventListener(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingLogger(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingLogger(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezSdk(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezSdk(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezServer(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezServer(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerDepositClaimError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerDepositClaimError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerEventEmitter(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerEventEmitter(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerFee(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerFee(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerParseError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerParseError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPath(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPath(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPaymentDetails(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPaymentDetails(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultChainServiceErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultChainServiceErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultOptionStringStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultOptionStringStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultPaymentStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultPaymentStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultRestResponseServiceConnectivityErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultRestResponseServiceConnectivityErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStringChainServiceErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStringChainServiceErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecDepositInfoStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecDepositInfoStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecFiatCurrencyServiceConnectivityErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecFiatCurrencyServiceConnectivityErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecPaymentStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecPaymentStorageErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecRateServiceConnectivityErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecRateServiceConnectivityErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecUtxoChainServiceErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecUtxoChainServiceErrorSendasync_trait(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPrepareSendPaymentResponse(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPrepareSendPaymentResponse(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentMethod(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentMethod(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentRequest(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentRequest(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRefundDepositRequest(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRefundDepositRequest(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReqwestRestClient(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReqwestRestClient(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRestClientChainService(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRestClientChainService(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkBuilder(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkBuilder(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkEvent(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkEvent(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentMethod(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentMethod(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentOptions(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentOptions(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentRequest(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentRequest(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerServiceConnectivityError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerServiceConnectivityError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSqliteStorage(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSqliteStorage(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerStorageError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerStorageError(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessAction(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessAction(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessActionProcessed(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessActionProcessed(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerUpdateDepositPayload(const void *ptr);

void frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerUpdateDepositPayload(const void *ptr);

struct wire_cst_event_listener_implementor *frbgen_breez_sdk_spark_cst_new_box_DynTrait_EventListener(void);

struct wire_cst_logger_implementor *frbgen_breez_sdk_spark_cst_new_box_DynTrait_Logger(void);

struct wire_cst_rest_client_implementor *frbgen_breez_sdk_spark_cst_new_box_DynTrait_RestClient(void);

uintptr_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBasicAuth(uintptr_t value);

uintptr_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerDepositClaimError(uintptr_t value);

uintptr_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerFee(uintptr_t value);

uintptr_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPaymentDetails(uintptr_t value);

uintptr_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentOptions(uintptr_t value);

uintptr_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessAction(uintptr_t value);

uintptr_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessActionProcessed(uintptr_t value);

struct wire_cst_binding_event_listener *frbgen_breez_sdk_spark_cst_new_box_autoadd_binding_event_listener(void);

struct wire_cst_binding_logger *frbgen_breez_sdk_spark_cst_new_box_autoadd_binding_logger(void);

struct wire_cst_bolt_11_invoice_details *frbgen_breez_sdk_spark_cst_new_box_autoadd_bolt_11_invoice_details(void);

bool *frbgen_breez_sdk_spark_cst_new_box_autoadd_bool(bool value);

struct wire_cst_claim_deposit_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_claim_deposit_request(void);

struct wire_cst_config *frbgen_breez_sdk_spark_cst_new_box_autoadd_config(void);

struct wire_cst_connect_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_connect_request(void);

struct wire_cst_event_listener_implementor *frbgen_breez_sdk_spark_cst_new_box_autoadd_event_listener_implementor(void);

struct wire_cst_get_info_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_get_info_request(void);

struct wire_cst_get_payment_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_get_payment_request(void);

struct wire_cst_list_payments_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_list_payments_request(void);

struct wire_cst_list_unclaimed_deposits_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_list_unclaimed_deposits_request(void);

struct wire_cst_lnurl_pay_info *frbgen_breez_sdk_spark_cst_new_box_autoadd_lnurl_pay_info(void);

struct wire_cst_lnurl_pay_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_lnurl_pay_request(void);

struct wire_cst_log_entry *frbgen_breez_sdk_spark_cst_new_box_autoadd_log_entry(void);

struct wire_cst_logger_implementor *frbgen_breez_sdk_spark_cst_new_box_autoadd_logger_implementor(void);

struct wire_cst_payment *frbgen_breez_sdk_spark_cst_new_box_autoadd_payment(void);

struct wire_cst_payment_metadata *frbgen_breez_sdk_spark_cst_new_box_autoadd_payment_metadata(void);

struct wire_cst_prepare_lnurl_pay_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_prepare_lnurl_pay_request(void);

struct wire_cst_prepare_send_payment_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_prepare_send_payment_request(void);

struct wire_cst_rest_client_implementor *frbgen_breez_sdk_spark_cst_new_box_autoadd_rest_client_implementor(void);

struct wire_cst_send_onchain_speed_fee_quote *frbgen_breez_sdk_spark_cst_new_box_autoadd_send_onchain_speed_fee_quote(void);

struct wire_cst_sync_wallet_request *frbgen_breez_sdk_spark_cst_new_box_autoadd_sync_wallet_request(void);

uint32_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_u_32(uint32_t value);

uint64_t *frbgen_breez_sdk_spark_cst_new_box_autoadd_u_64(uint64_t value);

struct wire_cst_list_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType *frbgen_breez_sdk_spark_cst_new_list_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType(int32_t len);

struct wire_cst_list_bip_21_extra *frbgen_breez_sdk_spark_cst_new_list_bip_21_extra(int32_t len);

struct wire_cst_list_bolt_11_route_hint *frbgen_breez_sdk_spark_cst_new_list_bolt_11_route_hint(int32_t len);

struct wire_cst_list_bolt_11_route_hint_hop *frbgen_breez_sdk_spark_cst_new_list_bolt_11_route_hint_hop(int32_t len);

struct wire_cst_list_deposit_info *frbgen_breez_sdk_spark_cst_new_list_deposit_info(int32_t len);

struct wire_cst_list_payment *frbgen_breez_sdk_spark_cst_new_list_payment(int32_t len);

struct wire_cst_list_prim_u_8_strict *frbgen_breez_sdk_spark_cst_new_list_prim_u_8_strict(int32_t len);

struct wire_cst_list_record_string_string *frbgen_breez_sdk_spark_cst_new_list_record_string_string(int32_t len);
static int64_t dummy_method_to_enforce_bundling(void) {
    int64_t dummy_var = 0;
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_DynTrait_EventListener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_DynTrait_Logger);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_DynTrait_RestClient);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBasicAuth);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerDepositClaimError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerFee);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPaymentDetails);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentOptions);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessAction);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessActionProcessed);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_binding_event_listener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_binding_logger);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_bolt_11_invoice_details);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_bool);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_claim_deposit_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_config);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_connect_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_event_listener_implementor);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_get_info_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_get_payment_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_list_payments_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_list_unclaimed_deposits_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_lnurl_pay_info);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_lnurl_pay_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_log_entry);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_logger_implementor);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_payment);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_payment_metadata);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_prepare_lnurl_pay_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_prepare_send_payment_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_rest_client_implementor);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_send_onchain_speed_fee_quote);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_sync_wallet_request);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_u_32);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_box_autoadd_u_64);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_Auto_Owned_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_bip_21_extra);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_bolt_11_route_hint);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_bolt_11_route_hint_hop);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_deposit_info);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_payment);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_prim_u_8_strict);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_cst_new_list_record_string_string);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerArcdynStorage);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBasicAuth);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingEventListener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingLogger);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezSdk);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezServer);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerDepositClaimError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerEventEmitter);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerFee);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerParseError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPath);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPaymentDetails);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultChainServiceErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultOptionStringStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultPaymentStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultRestResponseServiceConnectivityErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStringChainServiceErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecDepositInfoStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecFiatCurrencyServiceConnectivityErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecPaymentStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecRateServiceConnectivityErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecUtxoChainServiceErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPrepareSendPaymentResponse);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentMethod);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentRequest);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRefundDepositRequest);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReqwestRestClient);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRestClientChainService);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkBuilder);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkEvent);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentMethod);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentOptions);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentRequest);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerServiceConnectivityError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSqliteStorage);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerStorageError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessAction);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessActionProcessed);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_decrement_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerUpdateDepositPayload);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerArcdynStorage);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBasicAuth);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingEventListener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBindingLogger);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezSdk);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerBreezServer);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerDepositClaimError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerEventEmitter);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerFee);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerInputType);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerParseError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPath);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPaymentDetails);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultChainServiceErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultOptionStringStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultPaymentStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultRestResponseServiceConnectivityErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultStringChainServiceErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecDepositInfoStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecFiatCurrencyServiceConnectivityErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecPaymentStorageErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecRateServiceConnectivityErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPinBoxFutureOutputResultVecUtxoChainServiceErrorSendasync_trait);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerPrepareSendPaymentResponse);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentMethod);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReceivePaymentRequest);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRefundDepositRequest);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerReqwestRestClient);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerRestClientChainService);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkBuilder);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSdkEvent);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentMethod);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentOptions);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSendPaymentRequest);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerServiceConnectivityError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSqliteStorage);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerStorageError);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessAction);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerSuccessActionProcessed);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_rust_arc_increment_strong_count_RustOpaque_flutter_rust_bridgefor_generatedRustAutoOpaqueInnerUpdateDepositPayload);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__breez_server__BreezServer_fetch_fiat_currencies);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__breez_server__BreezServer_fetch_fiat_rates);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bip_21_details_default);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bip_21_extra_default);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bolt_11_route_hint_default);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__input__bolt_11_route_hint_hop_default);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__input__parse);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__input__parse_invoice);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__input__payment_request_source_default);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__rest__rest_client__ReqwestRestClient_get);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__rest__rest_client__ReqwestRestClient_new);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_common__rest__rest_client__ReqwestRestClient_post);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_claim_deposit);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_disconnect);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_frb_override_add_event_listener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_get_info);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_get_payment);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_list_payments);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_list_unclaimed_deposits);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_lnurl_pay);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_prepare_lnurl_pay);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_prepare_send_payment);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_receive_payment);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_refund_deposit);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_remove_event_listener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_send_payment);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__BreezSdk_sync_wallet);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_add_listener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_default);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_emit);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_new);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__EventEmitter_remove_listener);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SdkBuilder_build);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SdkBuilder_new);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_add_deposit);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_delete_deposit);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_get_cached_item);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_get_payment_by_id);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_insert_payment);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_list_deposits);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_list_payments);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_new);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_set_cached_item);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_set_payment_metadata);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__SqliteStorage_update_deposit);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__BasicAuth_new);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_broadcast_transaction);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_get_address_utxos);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_get_transaction_hex);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__chain__rest_client__RestClientChainService_new);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__connect);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__default_config);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__default_storage);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__init_logging);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__Fee_to_sats);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_get_amount_sats);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_get_payment_method);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_set_amount_sats);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__PrepareSendPaymentResponse_auto_accessor_set_payment_method);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__ReceivePaymentRequest_auto_accessor_get_payment_method);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__ReceivePaymentRequest_auto_accessor_set_payment_method);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_destination_address);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_fee);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_txid);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_get_vout);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_destination_address);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_fee);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_txid);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__RefundDepositRequest_auto_accessor_set_vout);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_get_options);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_get_prepare_response);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_set_options);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__SendPaymentRequest_auto_accessor_set_prepare_response);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__lnurl_pay_info_default);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__models__send_onchain_speed_fee_quote_total_fee_sat);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__breez_sdk_spark__parse);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__crate__binding_event_listener_on_event);
    dummy_var ^= ((int64_t) (void*) frbgen_breez_sdk_spark_wire__crate__binding_logger_log);
    dummy_var ^= ((int64_t) (void*) store_dart_post_cobject);
    return dummy_var;
}
