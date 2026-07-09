import {
  type BreezSdk,
  type ExternalSparkSigner,
  type LnurlPayResponse,
  type Payment,
  type PrepareLnurlPayResponse,
  type PrepareSendPaymentResponse,
  type SignedTransferPackage,
  type TransferSignature,
  type UnsignedTransferPackage
} from '@breeztech/breez-sdk-spark'

const signPackage = async (
  signer: ExternalSparkSigner,
  unsigned: UnsignedTransferPackage
): Promise<SignedTransferPackage> => {
  // ANCHOR: client-signing-sign-package
  let signature: TransferSignature
  switch (unsigned.type) {
    case 'transfer': {
      const { prepareTransfer, amountSat, feeSat, target } = unsigned
      // Show the user what they are approving before signing
      const destination = target.type === 'lightning' ? target.bolt11 : target.address
      console.log(`Approve sending ${amountSat} sats (fee ${feeSat} sats) to ${destination}`)
      signature = {
        type: 'transfer',
        signed: await signer.prepareTransfer(prepareTransfer)
      }
      break
    }
    case 'swap': {
      const { prepareTransfer, amountSat, feeSat } = unsigned
      console.log(`Approve re-shaping funds for a ${amountSat} sat send (fee ${feeSat} sats)`)
      signature = {
        type: 'transfer',
        signed: await signer.prepareTransfer(prepareTransfer)
      }
      break
    }
    case 'token': {
      const { prepareTokenTransaction, tokenIdentifier, amount, fee } = unsigned
      console.log(`Approve sending ${amount} of token ${tokenIdentifier} (fee ${fee})`)
      signature = {
        type: 'token',
        signed: await signer.prepareTokenTransaction(prepareTokenTransaction)
      }
      break
    }
  }

  const signedPackage = { unsigned, signature }
  // ANCHOR_END: client-signing-sign-package
  return signedPackage
}

const sendWithClientSigning = async (
  sdk: BreezSdk,
  signer: ExternalSparkSigner
): Promise<Payment> => {
  // ANCHOR: client-signing-send
  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest: { type: 'input', input: '<spark address or invoice>' },
    amount: BigInt(5_000),
    tokenIdentifier: undefined,
    conversionOptions: undefined,
    feePolicy: undefined
  })

  while (true) {
    const unsigned = await sdk.buildUnsignedTransferPackage({
      prepareResponse,
      options: undefined
    })

    // Send the package to the user, who reviews and signs it
    const signedPackage = await signPackage(signer, unsigned)

    const publishResponse = await sdk.publishSignedTransferPackage({ signedPackage })

    if (publishResponse.type === 'swapCompleted') {
      // The wallet's funds were re-shaped first: build the payment again
      continue
    }
    return publishResponse.payment
  }
  // ANCHOR_END: client-signing-send
}

const buildOnchainPackage = async (
  sdk: BreezSdk,
  prepareResponse: PrepareSendPaymentResponse
) => {
  // ANCHOR: client-signing-build-onchain-options
  // For Bitcoin address sends, the confirmation speed is chosen when
  // building the package: the fee depends on it
  const unsigned = await sdk.buildUnsignedTransferPackage({
    prepareResponse,
    options: {
      type: 'bitcoinAddress',
      confirmationSpeed: 'medium'
    }
  })
  // ANCHOR_END: client-signing-build-onchain-options
  console.log(unsigned)
}

const buildBolt11Package = async (
  sdk: BreezSdk,
  prepareResponse: PrepareSendPaymentResponse
) => {
  // ANCHOR: client-signing-build-bolt11-options
  const unsigned = await sdk.buildUnsignedTransferPackage({
    prepareResponse,
    options: {
      type: 'bolt11Invoice',
      preferSpark: true,
      completionTimeoutSecs: 10
    }
  })
  // ANCHOR_END: client-signing-build-bolt11-options
  console.log(unsigned)
}

const lnurlPayWithClientSigning = async (
  sdk: BreezSdk,
  signer: ExternalSparkSigner,
  prepareResponse: PrepareLnurlPayResponse
): Promise<LnurlPayResponse> => {
  // ANCHOR: client-signing-lnurl-pay
  while (true) {
    const unsigned = await sdk.buildUnsignedLnurlPayPackage({ prepareResponse })

    const signedPackage = await signPackage(signer, unsigned)

    const publishResponse = await sdk.publishSignedLnurlPayPackage({ signedPackage })

    if (publishResponse.type === 'swapCompleted') {
      continue
    }
    return publishResponse.response
  }
  // ANCHOR_END: client-signing-lnurl-pay
}
