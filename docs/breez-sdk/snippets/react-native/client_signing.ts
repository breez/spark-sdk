import {
  BuildTransferPackageOptions,
  OnchainConfirmationSpeed,
  PaymentRequest,
  PublishSignedLnurlPayResponse_Tags,
  PublishSignedTransferPackageResponse_Tags,
  TransferSignature,
  TransferTarget_Tags,
  UnsignedTransferPackage_Tags,
  type BreezSdk,
  type ExternalSparkSigner,
  type LnurlPayResponse,
  type Payment,
  type PrepareLnurlPayResponse,
  type PrepareSendPaymentResponse,
  type SignedTransferPackage,
  type UnsignedTransferPackage
} from '@breeztech/breez-sdk-spark-react-native'

const signPackage = async (
  signer: ExternalSparkSigner,
  unsigned: UnsignedTransferPackage
): Promise<SignedTransferPackage> => {
  // ANCHOR: client-signing-sign-package
  let signature: TransferSignature
  switch (unsigned.tag) {
    case UnsignedTransferPackage_Tags.Transfer: {
      const { prepareTransfer, amountSat, feeSat, target } = unsigned.inner
      // Show the user what they are approving before signing
      const destination =
        target.tag === TransferTarget_Tags.Lightning ? target.inner.bolt11 : target.inner.address
      console.log(`Approve sending ${amountSat} sats (fee ${feeSat} sats) to ${destination}`)
      signature = new TransferSignature.Transfer({
        signed: await signer.prepareTransfer(prepareTransfer)
      })
      break
    }
    case UnsignedTransferPackage_Tags.Swap: {
      const { prepareTransfer, amountSat, feeSat } = unsigned.inner
      console.log(`Approve re-shaping funds for a ${amountSat} sat send (fee ${feeSat} sats)`)
      signature = new TransferSignature.Transfer({
        signed: await signer.prepareTransfer(prepareTransfer)
      })
      break
    }
    case UnsignedTransferPackage_Tags.Token: {
      const { prepareTokenTransaction, tokenIdentifier, amount, fee, isSwap } = unsigned.inner
      if (isSwap) {
        console.log(`Approve combining token outputs for a ${tokenIdentifier} send`)
      } else {
        console.log(`Approve sending ${amount} of token ${tokenIdentifier} (fee ${fee})`)
      }
      signature = new TransferSignature.Token({
        signed: await signer.prepareTokenTransaction(prepareTokenTransaction)
      })
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
    paymentRequest: new PaymentRequest.Input({ input: '<spark address or invoice>' }),
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

    if (publishResponse.tag === PublishSignedTransferPackageResponse_Tags.SwapCompleted) {
      // The wallet's funds were re-shaped first: build the payment again
      continue
    }
    return publishResponse.inner.payment
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
    options: new BuildTransferPackageOptions.BitcoinAddress({
      confirmationSpeed: OnchainConfirmationSpeed.Medium
    })
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
    options: new BuildTransferPackageOptions.Bolt11Invoice({
      preferSpark: true,
      completionTimeoutSecs: 10
    })
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

    if (publishResponse.tag === PublishSignedLnurlPayResponse_Tags.SwapCompleted) {
      continue
    }
    return publishResponse.inner.response
  }
  // ANCHOR_END: client-signing-lnurl-pay
}

export {
  signPackage,
  sendWithClientSigning,
  buildOnchainPackage,
  buildBolt11Package,
  lnurlPayWithClientSigning
}
