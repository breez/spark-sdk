package example

import (
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func SignPackage(
	signer breez_sdk_spark.ExternalSparkSigner,
	unsigned breez_sdk_spark.UnsignedTransferPackage,
) (breez_sdk_spark.SignedTransferPackage, error) {
	// ANCHOR: client-signing-sign-package
	var signature breez_sdk_spark.TransferSignature

	switch pkg := unsigned.(type) {
	case breez_sdk_spark.UnsignedTransferPackageTransfer:
		// Show the user what they are approving before signing
		var destination string
		switch target := pkg.Target.(type) {
		case breez_sdk_spark.TransferTargetSpark:
			destination = target.Address
		case breez_sdk_spark.TransferTargetLightning:
			destination = target.Bolt11
		case breez_sdk_spark.TransferTargetCoopExit:
			destination = target.Address
		}
		log.Printf(
			"Approve sending %v sats (fee %v sats) to %v",
			pkg.AmountSat,
			pkg.FeeSat,
			destination,
		)
		signed, err := signer.PrepareTransfer(pkg.PrepareTransfer)
		if err != nil {
			return breez_sdk_spark.SignedTransferPackage{}, err
		}
		signature = breez_sdk_spark.TransferSignatureTransfer{Signed: signed}
	case breez_sdk_spark.UnsignedTransferPackageSwap:
		log.Printf(
			"Approve re-shaping funds for a %v sat send (fee %v sats)",
			pkg.AmountSat,
			pkg.FeeSat,
		)
		signed, err := signer.PrepareTransfer(pkg.PrepareTransfer)
		if err != nil {
			return breez_sdk_spark.SignedTransferPackage{}, err
		}
		signature = breez_sdk_spark.TransferSignatureTransfer{Signed: signed}
	case breez_sdk_spark.UnsignedTransferPackageToken:
		log.Printf(
			"Approve sending %v of token %v (fee %v)",
			pkg.Amount,
			pkg.TokenIdentifier,
			pkg.Fee,
		)
		signed, err := signer.PrepareTokenTransaction(pkg.PrepareTokenTransaction)
		if err != nil {
			return breez_sdk_spark.SignedTransferPackage{}, err
		}
		signature = breez_sdk_spark.TransferSignatureToken{Signed: signed}
	}

	signedPackage := breez_sdk_spark.SignedTransferPackage{
		Unsigned:  unsigned,
		Signature: signature,
	}
	// ANCHOR_END: client-signing-sign-package
	return signedPackage, nil
}

func SendWithClientSigning(
	sdk *breez_sdk_spark.BreezSdk,
	signer breez_sdk_spark.ExternalSparkSigner,
) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: client-signing-send
	paymentRequest := "<spark address or invoice>"
	amountSats := new(big.Int).SetInt64(5_000)

	prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
		Amount:            &amountSats,
		TokenIdentifier:   nil,
		ConversionOptions: nil,
		FeePolicy:         nil,
	})
	if err != nil {
		return nil, err
	}

	for {
		unsigned, err := sdk.BuildUnsignedTransferPackage(
			breez_sdk_spark.BuildUnsignedTransferPackageRequest{
				PrepareResponse: prepareResponse,
				Options:         nil,
			},
		)
		if err != nil {
			return nil, err
		}

		// Send the package to the user, who reviews and signs it
		signedPackage, err := SignPackage(signer, unsigned)
		if err != nil {
			return nil, err
		}

		response, err := sdk.PublishSignedTransferPackage(
			breez_sdk_spark.PublishSignedTransferPackageRequest{
				SignedPackage: signedPackage,
			},
		)
		if err != nil {
			return nil, err
		}

		switch result := response.(type) {
		// The wallet's funds were re-shaped first: build the payment again
		case breez_sdk_spark.PublishSignedTransferPackageResponseSwapCompleted:
			continue
		case breez_sdk_spark.PublishSignedTransferPackageResponsePaymentSent:
			return &result.Payment, nil
		}
	}
	// ANCHOR_END: client-signing-send
}

func BuildOnchainPackage(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) error {
	// ANCHOR: client-signing-build-onchain-options
	// For Bitcoin address sends, the confirmation speed is chosen when
	// building the package: the fee depends on it
	var options breez_sdk_spark.BuildTransferPackageOptions
	options = breez_sdk_spark.BuildTransferPackageOptionsBitcoinAddress{
		ConfirmationSpeed: breez_sdk_spark.OnchainConfirmationSpeedMedium,
	}

	unsigned, err := sdk.BuildUnsignedTransferPackage(
		breez_sdk_spark.BuildUnsignedTransferPackageRequest{
			PrepareResponse: prepareResponse,
			Options:         &options,
		},
	)
	if err != nil {
		return err
	}
	// ANCHOR_END: client-signing-build-onchain-options
	log.Printf("Unsigned package: %v", unsigned)
	return nil
}

func BuildBolt11Package(sdk *breez_sdk_spark.BreezSdk, prepareResponse breez_sdk_spark.PrepareSendPaymentResponse) error {
	// ANCHOR: client-signing-build-bolt11-options
	var completionTimeoutSecs uint32 = 10
	var options breez_sdk_spark.BuildTransferPackageOptions
	options = breez_sdk_spark.BuildTransferPackageOptionsBolt11Invoice{
		PreferSpark:           true,
		CompletionTimeoutSecs: &completionTimeoutSecs,
	}

	unsigned, err := sdk.BuildUnsignedTransferPackage(
		breez_sdk_spark.BuildUnsignedTransferPackageRequest{
			PrepareResponse: prepareResponse,
			Options:         &options,
		},
	)
	if err != nil {
		return err
	}
	// ANCHOR_END: client-signing-build-bolt11-options
	log.Printf("Unsigned package: %v", unsigned)
	return nil
}

func LnurlPayWithClientSigning(
	sdk *breez_sdk_spark.BreezSdk,
	signer breez_sdk_spark.ExternalSparkSigner,
	prepareResponse breez_sdk_spark.PrepareLnurlPayResponse,
) (*breez_sdk_spark.LnurlPayResponse, error) {
	// ANCHOR: client-signing-lnurl-pay
	for {
		unsigned, err := sdk.BuildUnsignedLnurlPayPackage(
			breez_sdk_spark.BuildUnsignedLnurlPayPackageRequest{
				PrepareResponse: prepareResponse,
			},
		)
		if err != nil {
			return nil, err
		}

		signedPackage, err := SignPackage(signer, unsigned)
		if err != nil {
			return nil, err
		}

		response, err := sdk.PublishSignedLnurlPayPackage(
			breez_sdk_spark.PublishSignedLnurlPayPackageRequest{
				SignedPackage: signedPackage,
			},
		)
		if err != nil {
			return nil, err
		}

		switch result := response.(type) {
		case breez_sdk_spark.PublishSignedLnurlPayResponseSwapCompleted:
			continue
		case breez_sdk_spark.PublishSignedLnurlPayResponsePaymentSent:
			return &result.Response, nil
		}
	}
	// ANCHOR_END: client-signing-lnurl-pay
}
