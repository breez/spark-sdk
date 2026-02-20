import React from 'react'
import { Alert } from 'react-native'
import type { BreezSdk } from '@breeztech/breez-sdk-spark-react-native'
import {
  WebLnWebView,
  type LnurlRequest,
  type LnurlUserResponse
} from '@breeztech/breez-sdk-spark-react-native/webln'

const WebLnExample: React.FC<{ sdk: BreezSdk, uri: string }> = ({ sdk, uri }) => {
  return (
    // ANCHOR: webln-integration
    <WebLnWebView
      sdk={sdk}
      source={{ uri }}
      onEnableRequest={async (domain: string) => {
        // Show a dialog asking the user to approve WebLN access
        return await new Promise((resolve) => {
          Alert.alert('WebLN', `Allow ${domain} to connect?`, [
            { text: 'Deny', onPress: () => { resolve(false) } },
            { text: 'Allow', onPress: () => { resolve(true) } }
          ])
        })
      }}
      onPaymentRequest={async (invoice: string, amountSats: number) => {
        // Show a dialog asking the user to approve the payment
        return await new Promise((resolve) => {
          Alert.alert('Payment Request', `Pay ${amountSats} sats?`, [
            { text: 'Deny', onPress: () => { resolve(false) } },
            { text: 'Pay', onPress: () => { resolve(true) } }
          ])
        })
      }}
      onLnurlRequest={async (request: LnurlRequest): Promise<LnurlUserResponse> => {
        // Handle LNURL requests (pay, withdraw, auth)
        switch (request.type) {
          case 'pay':
            // Show UI to select amount within min/max bounds
            // Return LnurlUserResponse with approved, amountSats, and optional comment
            return { approved: true, amountSats: 1000 }
          case 'withdraw':
            // Show UI to select amount within min/max bounds
            return { approved: true, amountSats: 1000 }
          case 'auth':
            // Show confirmation dialog
            return { approved: true }
        }
      }}
    />
    // ANCHOR_END: webln-integration
  )
}

export default WebLnExample
