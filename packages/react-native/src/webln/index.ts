/**
 * WebLn support for React Native WebViews
 *
 * This module provides WebLN integration for React Native apps using
 * `react-native-webview`. It allows WebLN-aware websites to interact
 * with the Breez Spark SDK through a WebView bridge.
 *
 * ## Installation
 *
 * This module requires `react-native-webview` as a peer dependency:
 * ```bash
 * npm install react-native-webview
 * ```
 *
 * ## Usage
 *
 * ```tsx
 * import { WebLnWebView } from '@breeztech/breez-sdk-spark-react-native/webln';
 *
 * <WebLnWebView
 *   sdk={sdk}
 *   source={{ uri: 'https://example.com' }}
 *   onEnableRequest={async (domain) => {
 *     return await confirmEnable(domain);
 *   }}
 *   onPaymentRequest={async (invoice, amountSats) => {
 *     return await confirmPayment(invoice, amountSats);
 *   }}
 *   onLnurlRequest={async (request) => {
 *     return await handleLnurl(request);
 *   }}
 * />
 * ```
 */

export * from './types';
export * from './WebLnWebView';
export { weblnProviderScript } from './providerScript';
