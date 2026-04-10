/**
 * Ambient module declaration for the Breez SDK React Native package.
 *
 * In CI the SDK's generated bindings (produced by bob build) aren't available,
 * so we declare the module to let tsc verify the CLI's own code.  All SDK
 * imports resolve to `any`, which is acceptable: the goal of check-react-native
 * is to catch errors *inside* the CLI, not to type-check the SDK itself.
 */
declare module '@breeztech/breez-sdk-spark-react-native' {
  const _default: any;
  export default _default;

  // --- values + types (used as both `Network.Regtest` and `: Network`) ---
  export const Network: any;
  export type Network = any;

  export const Seed: any;
  export type Seed = any;

  export const SdkBuilder: any;
  export type SdkBuilder = any;

  export const PaymentType: any;
  export type PaymentType = any;

  export const PaymentStatus: any;
  export type PaymentStatus = any;

  export const AssetFilter: any;
  export type AssetFilter = any;

  export const PaymentDetailsFilter: any;
  export type PaymentDetailsFilter = any;

  export const SparkHtlcStatus: any;
  export type SparkHtlcStatus = any;

  export const ConversionOptions: any;
  export type ConversionOptions = any;

  export const ConversionType: any;
  export type ConversionType = any;

  export const MaxFee: any;
  export type MaxFee = any;

  export const Fee: any;
  export type Fee = any;

  export const FeePolicy: any;
  export type FeePolicy = any;

  export const TokenTransactionType: any;
  export type TokenTransactionType = any;

  export const BuyBitcoinRequest: any;
  export type BuyBitcoinRequest = any;

  export const WebhookEventType: any;
  export type WebhookEventType = any;

  // --- values only ---
  export const defaultConfig: any;
  export const SdkEvent_Tags: any;
  export const InputType_Tags: any;
  export const ReceivePaymentMethod: any;
  export const SendPaymentMethod_Tags: any;
  export const SendPaymentOptions: any;
  export const OnchainConfirmationSpeed: any;
  export const getSparkStatus: any;

  // --- passkey ---
  export const PasskeyPrfProvider: any;
  export type PasskeyPrfProvider = any;

  export const Passkey: any;
  export type Passkey = any;

  export const NostrRelayConfig: any;
  export type NostrRelayConfig = any;

  // --- types only ---
  export type BreezSdkInterface = any;
  export type TokenIssuerInterface = any;
  export type SdkEvent = any;
}
