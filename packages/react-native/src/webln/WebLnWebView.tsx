/**
 * WebLn-enabled WebView component for React Native
 *
 * This component wraps react-native-webview to provide WebLN support,
 * allowing WebLN-aware websites to interact with the Breez Spark SDK.
 */

import React, { useCallback, useRef, useImperativeHandle } from 'react';
import type { ComponentProps } from 'react';

// Types from the SDK
import type { BreezSdkInterface } from '../generated/breez_sdk_spark';
import {
  InputType,
  SdkError,
  PrepareSendPaymentRequest,
  SendPaymentRequest,
  SendPaymentOptions,
  ReceivePaymentRequest,
  ReceivePaymentMethod,
  SignMessageRequest,
  CheckMessageRequest,
  PrepareLnurlPayRequest,
  LnurlPayRequest,
  LnurlWithdrawRequest,
  PaymentDetails,
} from '../generated/breez_sdk_spark';

import type {
  WebLnRequest,
  WebLnResponse,
  OnEnableRequest,
  OnPaymentRequest,
  OnLnurlRequest,
} from './types';
import { WebLnErrorCode } from './types';
import { weblnProviderScript } from './providerScript';

// Note: react-native-webview is a peer dependency
// Users must install it separately: npm install react-native-webview
type WebViewType = React.ComponentType<{
  ref?: React.Ref<unknown>;
  source: { uri: string } | { html: string };
  injectedJavaScriptBeforeContentLoaded?: string;
  onMessage?: (event: { nativeEvent: { data: string } }) => void;
  javaScriptEnabled?: boolean;
  [key: string]: unknown;
}>;

// Try to import WebView, but allow it to be undefined if not installed
let WebView: WebViewType | undefined;
try {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  WebView = require('react-native-webview').WebView;
} catch {
  // WebView not installed - will throw helpful error if component is used
}

/**
 * Props for WebLN callbacks.
 * These callbacks are invoked during WebLN operations to get user approval.
 */
export interface WebLnCallbacks {
  /** Callback when a website requests WebLN access */
  onEnableRequest: OnEnableRequest;
  /** Callback when a website requests payment approval */
  onPaymentRequest: OnPaymentRequest;
  /** Callback when a website initiates an LNURL flow */
  onLnurlRequest: OnLnurlRequest;
}

/** Props for the WebLnWebView component */
export interface WebLnWebViewProps extends WebLnCallbacks {
  /** The Breez SDK instance */
  sdk: BreezSdkInterface;
  /** The URL or HTML to load */
  source: { uri: string } | { html: string };
  /** Additional props passed to the underlying WebView */
  webViewProps?: Omit<
    ComponentProps<WebViewType>,
    'source' | 'injectedJavaScript' | 'onMessage' | 'ref'
  >;
}

/** Handle for the WebLnWebView component */
export interface WebLnWebViewHandle {
  /** Reference to the underlying WebView */
  webViewRef: React.RefObject<unknown>;
}

/** Supported WebLN methods */
const SUPPORTED_METHODS = [
  'getInfo',
  'sendPayment',
  'makeInvoice',
  'signMessage',
  'verifyMessage',
  'lnurl',
];

/**
 * WebLn-enabled WebView component
 *
 * This component provides WebLN support in React Native WebViews,
 * allowing WebLN-aware websites to interact with the Breez Spark SDK.
 *
 * @example
 * ```tsx
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
export const WebLnWebView = React.forwardRef<
  WebLnWebViewHandle,
  WebLnWebViewProps
>(function WebLnWebView(
  { sdk, source, onEnableRequest, onPaymentRequest, onLnurlRequest, webViewProps },
  ref
) {
  if (!WebView) {
    throw new Error(
      'react-native-webview is not installed. Please install it: npm install react-native-webview'
    );
  }

  const webViewRef = useRef<unknown>(null);
  const enabledDomains = useRef(new Set<string>());
  const cachedPubkey = useRef<string | null>(null);

  useImperativeHandle(ref, () => ({ webViewRef }), []);

  /** Gets the node pubkey by signing a message */
  const getNodePubkey = useCallback(async (): Promise<string> => {
    if (cachedPubkey.current) {
      return cachedPubkey.current;
    }

    const response = await sdk.signMessage({
      message: 'webln_pubkey_request',
      compact: true,
    });
    cachedPubkey.current = response.pubkey;
    return response.pubkey;
  }, [sdk]);

  /** Sends a response back to the WebView */
  const respond = useCallback(
    (id: string, result?: Record<string, unknown>, error?: string) => {
      const response: WebLnResponse = {
        id,
        success: error === undefined,
        ...(result && { result }),
        ...(error && { error }),
      };

      const js = `window.__breezSparkWebLnHandleResponse(${JSON.stringify(response)});`;
      // @ts-expect-error - WebView ref type is complex
      webViewRef.current?.injectJavaScript?.(js);
    },
    []
  );

  /** Handles the enable request */
  const handleEnable = useCallback(
    async (id: string, params: Record<string, unknown>) => {
      const domain = params.domain as string | undefined;
      if (!domain) {
        respond(id, undefined, WebLnErrorCode.invalidParams);
        return;
      }

      if (enabledDomains.current.has(domain)) {
        respond(id, {});
        return;
      }

      const approved = await onEnableRequest(domain);
      if (approved) {
        enabledDomains.current.add(domain);
        respond(id, {});
      } else {
        respond(id, undefined, WebLnErrorCode.userRejected);
      }
    },
    [onEnableRequest, respond]
  );

  /** Handles getInfo request */
  const handleGetInfo = useCallback(
    async (id: string) => {
      try {
        const pubkey = await getNodePubkey();
        respond(id, {
          node: { pubkey, alias: '' },
          methods: SUPPORTED_METHODS,
        });
      } catch {
        respond(id, undefined, WebLnErrorCode.internalError);
      }
    },
    [getNodePubkey, respond]
  );

  /** Handles sendPayment request */
  const handleSendPayment = useCallback(
    async (id: string, params: Record<string, unknown>) => {
      const paymentRequest = params.paymentRequest as string | undefined;
      if (!paymentRequest) {
        respond(id, undefined, WebLnErrorCode.invalidParams);
        return;
      }

      try {
        // Parse the invoice to get amount
        const parsed = await sdk.parse(paymentRequest);
        let amountSats = 0;

        if (InputType.Bolt11Invoice.instanceOf(parsed)) {
          const invoiceDetails = parsed.inner[0];
          if (invoiceDetails.amountMsat !== undefined) {
            amountSats = Math.round(Number(invoiceDetails.amountMsat) / 1000);
          }
        }

        // Request payment confirmation from user
        const approved = await onPaymentRequest(paymentRequest, amountSats);
        if (!approved) {
          respond(id, undefined, WebLnErrorCode.userRejected);
          return;
        }

        // Prepare and send payment
        const prepared = await sdk.prepareSendPayment(
          PrepareSendPaymentRequest.create({ paymentRequest })
        );
        const result = await sdk.sendPayment(
          SendPaymentRequest.create({
            prepareResponse: prepared,
            options: new SendPaymentOptions.Bolt11Invoice({
              preferSpark: false,
              completionTimeoutSecs: 60,
            }),
          })
        );

        // Extract preimage from payment details
        let preimage = '';
        const details = result.payment.details;
        if (details && PaymentDetails.Lightning.instanceOf(details)) {
          preimage = details.inner.htlcDetails.preimage ?? '';
        }

        respond(id, { preimage });
      } catch (e) {
        if (SdkError.InsufficientFunds.instanceOf(e)) {
          respond(id, undefined, WebLnErrorCode.insufficientFunds);
        } else {
          respond(id, undefined, WebLnErrorCode.internalError);
        }
      }
    },
    [sdk, onPaymentRequest, respond]
  );

  /** Handles makeInvoice request */
  const handleMakeInvoice = useCallback(
    async (id: string, params: Record<string, unknown>) => {
      try {
        const amount = (params.amount ?? params.defaultAmount) as
          | number
          | string
          | undefined;
        const memo = params.defaultMemo as string | undefined;

        let amountSats: bigint | undefined;
        if (amount !== undefined) {
          if (typeof amount === 'number') {
            amountSats = BigInt(amount);
          } else if (typeof amount === 'string') {
            const parsed = parseInt(amount, 10);
            if (!isNaN(parsed)) {
              amountSats = BigInt(parsed);
            }
          }
        }

        const response = await sdk.receivePayment({
          paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
            description: memo ?? '',
            amountSats,
            expirySecs: undefined,
            paymentHash: undefined,
          }),
        });

        respond(id, { paymentRequest: response.paymentRequest });
      } catch {
        respond(id, undefined, WebLnErrorCode.internalError);
      }
    },
    [sdk, respond]
  );

  /** Handles signMessage request */
  const handleSignMessage = useCallback(
    async (id: string, params: Record<string, unknown>) => {
      const message = params.message as string | undefined;
      if (!message) {
        respond(id, undefined, WebLnErrorCode.invalidParams);
        return;
      }

      try {
        const response = await sdk.signMessage({ message, compact: true });
        respond(id, { message, signature: response.signature });
      } catch {
        respond(id, undefined, WebLnErrorCode.internalError);
      }
    },
    [sdk, respond]
  );

  /** Handles verifyMessage request */
  const handleVerifyMessage = useCallback(
    async (id: string, params: Record<string, unknown>) => {
      const signature = params.signature as string | undefined;
      const message = params.message as string | undefined;

      if (!signature || !message) {
        respond(id, undefined, WebLnErrorCode.invalidParams);
        return;
      }

      try {
        const pubkey = await getNodePubkey();
        const response = await sdk.checkMessage({ message, pubkey, signature });

        if (response.isValid) {
          respond(id, {});
        } else {
          respond(id, undefined, WebLnErrorCode.invalidParams);
        }
      } catch {
        respond(id, undefined, WebLnErrorCode.internalError);
      }
    },
    [sdk, getNodePubkey, respond]
  );

  /** Handles LNURL-Pay */
  const handleLnurlPay = useCallback(
    async (id: string, data: unknown) => {
      // Type assertion - data is LnurlPayRequestDetails
      const payData = data as {
        domain: string;
        minSendable: bigint;
        maxSendable: bigint;
        metadataStr: string;
      };

      const lnurlResponse = await onLnurlRequest({
        type: 'pay',
        domain: payData.domain,
        minAmountSats: Math.round(Number(payData.minSendable) / 1000),
        maxAmountSats: Math.round(Number(payData.maxSendable) / 1000),
        metadata: payData.metadataStr,
      });

      if (!lnurlResponse.approved) {
        respond(id, undefined, WebLnErrorCode.userRejected);
        return;
      }

      try {
        const prepared = await sdk.prepareLnurlPay(
          PrepareLnurlPayRequest.create({
            payRequest: payData as Parameters<typeof sdk.prepareLnurlPay>[0]['payRequest'],
            amountSats: BigInt(lnurlResponse.amountSats ?? 0),
            comment: lnurlResponse.comment,
          })
        );

        const result = await sdk.lnurlPay(
          LnurlPayRequest.create({ prepareResponse: prepared })
        );

        // Extract preimage from payment details
        let preimage = '';
        const details = result.payment.details;
        if (details && PaymentDetails.Lightning.instanceOf(details)) {
          preimage = details.inner.htlcDetails.preimage ?? '';
        }

        respond(id, { status: 'OK', preimage });
      } catch (e) {
        if (SdkError.InsufficientFunds.instanceOf(e)) {
          respond(id, undefined, WebLnErrorCode.insufficientFunds);
        } else {
          respond(id, undefined, WebLnErrorCode.internalError);
        }
      }
    },
    [sdk, onLnurlRequest, respond]
  );

  /** Handles LNURL-Withdraw */
  const handleLnurlWithdraw = useCallback(
    async (id: string, data: unknown) => {
      // Type assertion - data is LnurlWithdrawRequestDetails
      const withdrawData = data as {
        callback: string;
        minWithdrawable: bigint;
        maxWithdrawable: bigint;
        defaultDescription: string;
      };

      let domain: string;
      try {
        const url = new URL(withdrawData.callback);
        domain = url.host;
      } catch {
        domain = withdrawData.callback;
      }

      const lnurlResponse = await onLnurlRequest({
        type: 'withdraw',
        domain,
        minAmountSats: Math.round(Number(withdrawData.minWithdrawable) / 1000),
        maxAmountSats: Math.round(Number(withdrawData.maxWithdrawable) / 1000),
        defaultDescription: withdrawData.defaultDescription,
      });

      if (!lnurlResponse.approved) {
        respond(id, undefined, WebLnErrorCode.userRejected);
        return;
      }

      try {
        await sdk.lnurlWithdraw(
          LnurlWithdrawRequest.create({
            withdrawRequest: withdrawData as Parameters<typeof sdk.lnurlWithdraw>[0]['withdrawRequest'],
            amountSats: BigInt(lnurlResponse.amountSats ?? 0),
          })
        );
        respond(id, { status: 'OK' });
      } catch {
        respond(id, undefined, WebLnErrorCode.internalError);
      }
    },
    [sdk, onLnurlRequest, respond]
  );

  /** Handles LNURL-Auth */
  const handleLnurlAuth = useCallback(
    async (id: string, data: unknown) => {
      // Type assertion - data is LnurlAuthRequestDetails
      const authData = data as { domain: string; k1: string; url: string; action?: string };

      const lnurlResponse = await onLnurlRequest({
        type: 'auth',
        domain: authData.domain,
      });

      if (!lnurlResponse.approved) {
        respond(id, undefined, WebLnErrorCode.userRejected);
        return;
      }

      try {
        // lnurlAuth takes LnurlAuthRequestDetails directly
        await sdk.lnurlAuth(
          authData as Parameters<typeof sdk.lnurlAuth>[0]
        );
        respond(id, { status: 'OK' });
      } catch {
        respond(id, undefined, WebLnErrorCode.internalError);
      }
    },
    [sdk, onLnurlRequest, respond]
  );

  /** Handles LNURL request */
  const handleLnurl = useCallback(
    async (id: string, params: Record<string, unknown>) => {
      const lnurlString = params.lnurl as string | undefined;
      if (!lnurlString) {
        respond(id, undefined, WebLnErrorCode.invalidParams);
        return;
      }

      try {
        const parsed = await sdk.parse(lnurlString);

        if (InputType.LnurlPay.instanceOf(parsed)) {
          await handleLnurlPay(id, parsed.inner[0]);
        } else if (InputType.LnurlWithdraw.instanceOf(parsed)) {
          await handleLnurlWithdraw(id, parsed.inner[0]);
        } else if (InputType.LnurlAuth.instanceOf(parsed)) {
          await handleLnurlAuth(id, parsed.inner[0]);
        } else {
          respond(id, undefined, WebLnErrorCode.invalidParams);
        }
      } catch {
        respond(id, undefined, WebLnErrorCode.internalError);
      }
    },
    [sdk, handleLnurlPay, handleLnurlWithdraw, handleLnurlAuth, respond]
  );

  /** Handles incoming messages from the WebView */
  const handleMessage = useCallback(
    async (event: { nativeEvent: { data: string } }) => {
      try {
        const request: WebLnRequest = JSON.parse(event.nativeEvent.data);
        const { id, method, params } = request;

        switch (method) {
          case 'enable':
            await handleEnable(id, params);
            break;
          case 'getInfo':
            await handleGetInfo(id);
            break;
          case 'sendPayment':
            await handleSendPayment(id, params);
            break;
          case 'makeInvoice':
            await handleMakeInvoice(id, params);
            break;
          case 'signMessage':
            await handleSignMessage(id, params);
            break;
          case 'verifyMessage':
            await handleVerifyMessage(id, params);
            break;
          case 'lnurl':
            await handleLnurl(id, params);
            break;
          default:
            respond(id, undefined, WebLnErrorCode.unsupportedMethod);
        }
      } catch (e) {
        console.error('WebLN error:', e);
      }
    },
    [
      handleEnable,
      handleGetInfo,
      handleSendPayment,
      handleMakeInvoice,
      handleSignMessage,
      handleVerifyMessage,
      handleLnurl,
      respond,
    ]
  );

  return (
    <WebView
      ref={webViewRef}
      source={source}
      injectedJavaScriptBeforeContentLoaded={weblnProviderScript}
      onMessage={handleMessage}
      javaScriptEnabled={true}
      {...webViewProps}
    />
  );
});

export default WebLnWebView;
