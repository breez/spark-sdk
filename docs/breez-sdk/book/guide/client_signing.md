# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, `send_payment` works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with `prepare_send_payment`, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with `build_unsigned_transfer_package`. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with `publish_signed_transfer_package` to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns `PublishSignedTransferPackageResponse::SwapCompleted`, and you build again from the same prepare response. Repeat until publishing returns `PublishSignedTransferPackageResponse::PaymentSent`.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

## Signing on the user's side

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html

The user's side does not need a connected SDK, only a signer that holds the user's key: any `ExternalSparkSigner` implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with `prepare_transfer`, and Token packages with `prepare_token_transaction`:

### Rust

```rust
let signature = match &unsigned {
    UnsignedTransferPackage::Transfer {
        prepare_transfer,
        amount_sat,
        fee_sat,
        target,
    } => {
        // Show the user what they are approving before signing
        let destination = match target {
            TransferTarget::Spark { address, .. } => address,
            TransferTarget::Lightning { bolt11, .. } => bolt11,
            TransferTarget::CoopExit { address, .. } => address,
        };
        info!("Approve sending {amount_sat} sats (fee {fee_sat} sats) to {destination}");
        TransferSignature::Transfer {
            signed: signer.prepare_transfer(prepare_transfer.clone()).await?,
        }
    }
    UnsignedTransferPackage::Swap {
        prepare_transfer,
        amount_sat,
        fee_sat,
        ..
    } => {
        info!("Approve re-shaping funds for a {amount_sat} sat send (fee {fee_sat} sats)");
        TransferSignature::Transfer {
            signed: signer.prepare_transfer(prepare_transfer.clone()).await?,
        }
    }
    UnsignedTransferPackage::Token {
        prepare_token_transaction,
        token_identifier,
        amount,
        fee,
        is_swap,
        ..
    } => {
        if *is_swap {
            info!("Approve combining token outputs for a {token_identifier} send");
        } else {
            info!("Approve sending {amount} of token {token_identifier} (fee {fee})");
        }
        TransferSignature::Token {
            signed: signer
                .prepare_token_transaction(prepare_token_transaction.clone())
                .await?,
        }
    }
    UnsignedTransferPackage::TokenBatch {
        prepare_token_transaction,
        totals,
        is_swap,
        ..
    } => {
        if *is_swap {
            info!("Approve combining token outputs before the batch is sent");
        } else {
            for total in totals {
                // Unset would mean sats, which a batch cannot send yet
                let token_id = total.token_identifier.as_deref().unwrap_or_default();
                info!("Approve sending {} of token {token_id}", total.amount);
            }
        }
        TransferSignature::Token {
            signed: signer
                .prepare_token_transaction(prepare_token_transaction.clone())
                .await?,
        }
    }
};

let signed_package = SignedTransferPackage {
    unsigned,
    signature,
};
```

### Swift

```swift
let signature: TransferSignature
switch unsigned {
case let .transfer(prepareTransfer, amountSat, feeSat, target):
    // Show the user what they are approving before signing
    let destination: String
    switch target {
    case let .spark(address, _):
        destination = address
    case let .lightning(bolt11, _, _, _):
        destination = bolt11
    case let .coopExit(address, _, _):
        destination = address
    }
    print("Approve sending \(amountSat) sats (fee \(feeSat) sats) to \(destination)")
    signature = TransferSignature.transfer(
        signed: try await signer.prepareTransfer(request: prepareTransfer)
    )
case let .swap(prepareTransfer, _, amountSat, feeSat):
    print("Approve re-shaping funds for a \(amountSat) sat send (fee \(feeSat) sats)")
    signature = TransferSignature.transfer(
        signed: try await signer.prepareTransfer(request: prepareTransfer)
    )
case let .token(prepareTokenTransaction, _, tokenIdentifier, amount, fee, isSwap):
    if isSwap {
        print("Approve combining token outputs for a \(tokenIdentifier) send")
    } else {
        print("Approve sending \(amount) of token \(tokenIdentifier) (fee \(fee))")
    }
    signature = TransferSignature.token(
        signed: try await signer.prepareTokenTransaction(request: prepareTokenTransaction)
    )
case let .tokenBatch(prepareTokenTransaction, _, totals, isSwap):
    if isSwap {
        print("Approve combining token outputs before the batch is sent")
    } else {
        for total in totals {
            // Unset would mean sats, which a batch cannot send yet
            let tokenId = total.tokenIdentifier ?? ""
            print("Approve sending \(total.amount) of token \(tokenId)")
        }
    }
    signature = TransferSignature.token(
        signed: try await signer.prepareTokenTransaction(request: prepareTokenTransaction)
    )
}

let signedPackage = SignedTransferPackage(
    unsigned: unsigned,
    signature: signature
)
```

### Kotlin

```kotlin
val signature = when (unsigned) {
    is UnsignedTransferPackage.Transfer -> {
        // Show the user what they are approving before signing
        val destination = when (val target = unsigned.target) {
            is TransferTarget.Spark -> target.address
            is TransferTarget.Lightning -> target.bolt11
            is TransferTarget.CoopExit -> target.address
        }
        // Log.v("Breez", "Approve sending ${unsigned.amountSat} sats " +
        //     "(fee ${unsigned.feeSat} sats) to $destination")
        TransferSignature.Transfer(
            signer.prepareTransfer(unsigned.prepareTransfer)
        )
    }
    is UnsignedTransferPackage.Swap -> {
        // Log.v("Breez", "Approve re-shaping funds for a ${unsigned.amountSat} " +
        //     "sat send (fee ${unsigned.feeSat} sats)")
        TransferSignature.Transfer(
            signer.prepareTransfer(unsigned.prepareTransfer)
        )
    }
    is UnsignedTransferPackage.Token -> {
        if (unsigned.isSwap) {
            // Log.v("Breez", "Approve combining token outputs for a " +
            //     "${unsigned.tokenIdentifier} send")
        } else {
            // Log.v("Breez", "Approve sending ${unsigned.amount} of token " +
            //     "${unsigned.tokenIdentifier} (fee ${unsigned.fee})")
        }
        TransferSignature.Token(
            signer.prepareTokenTransaction(unsigned.prepareTokenTransaction)
        )
    }
    is UnsignedTransferPackage.TokenBatch -> {
        if (unsigned.isSwap) {
            // Log.v("Breez", "Approve combining token outputs before the batch is sent")
        } else {
            for (total in unsigned.totals) {
                // Log.v("Breez", "Approve sending ${total.amount} of token " +
                //     "${total.tokenIdentifier}")
            }
        }
        TransferSignature.Token(
            signer.prepareTokenTransaction(unsigned.prepareTokenTransaction)
        )
    }
}

val signedPackage = SignedTransferPackage(unsigned, signature)
```

### C#

```csharp
TransferSignature signature;
switch (unsigned)
{
    case UnsignedTransferPackage.Transfer transfer:
        // Show the user what they are approving before signing
        var destination = transfer.target switch
        {
            TransferTarget.Spark spark => spark.address,
            TransferTarget.Lightning lightning => lightning.bolt11,
            TransferTarget.CoopExit coopExit => coopExit.address,
            _ => throw new Exception("Unknown transfer target")
        };
        Console.WriteLine($"Approve sending {transfer.amountSat} sats " +
            $"(fee {transfer.feeSat} sats) to {destination}");
        signature = new TransferSignature.Transfer(
            signed: await signer.PrepareTransfer(transfer.prepareTransfer)
        );
        break;
    case UnsignedTransferPackage.Swap swap:
        Console.WriteLine("Approve re-shaping funds for a " +
            $"{swap.amountSat} sat send (fee {swap.feeSat} sats)");
        signature = new TransferSignature.Transfer(
            signed: await signer.PrepareTransfer(swap.prepareTransfer)
        );
        break;
    case UnsignedTransferPackage.Token token:
        if (token.isSwap)
        {
            Console.WriteLine("Approve combining token outputs for a " +
                $"{token.tokenIdentifier} send");
        }
        else
        {
            Console.WriteLine($"Approve sending {token.amount} of token " +
                $"{token.tokenIdentifier} (fee {token.fee})");
        }
        signature = new TransferSignature.Token(
            signed: await signer.PrepareTokenTransaction(token.prepareTokenTransaction)
        );
        break;
    case UnsignedTransferPackage.TokenBatch tokenBatch:
        if (tokenBatch.isSwap)
        {
            Console.WriteLine("Approve combining token outputs " +
                "before the batch is sent");
        }
        else
        {
            foreach (var total in tokenBatch.totals)
            {
                Console.WriteLine($"Approve sending {total.amount} of token " +
                    $"{total.tokenIdentifier}");
            }
        }
        signature = new TransferSignature.Token(
            signed: await signer.PrepareTokenTransaction(
                tokenBatch.prepareTokenTransaction)
        );
        break;
    default:
        throw new Exception("Unknown transfer package");
}

var signedPackage = new SignedTransferPackage(unsigned: unsigned, signature: signature);
```

### Javascript (Wasm)

```typescript
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
    const { prepareTokenTransaction, tokenIdentifier, amount, fee, isSwap } = unsigned
    if (isSwap) {
      console.log(`Approve combining token outputs for a ${tokenIdentifier} send`)
    } else {
      console.log(`Approve sending ${amount} of token ${tokenIdentifier} (fee ${fee})`)
    }
    signature = {
      type: 'token',
      signed: await signer.prepareTokenTransaction(prepareTokenTransaction)
    }
    break
  }
  case 'tokenBatch': {
    const { prepareTokenTransaction, totals, isSwap } = unsigned
    if (isSwap) {
      console.log('Approve combining token outputs before the batch is sent')
    } else {
      for (const total of totals) {
        console.log(`Approve sending ${total.amount} of token ${total.tokenIdentifier}`)
      }
    }
    signature = {
      type: 'token',
      signed: await signer.prepareTokenTransaction(prepareTokenTransaction)
    }
    break
  }
}

const signedPackage = { unsigned, signature }
```

### React Native

```typescript
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
  case UnsignedTransferPackage_Tags.TokenBatch: {
    const { prepareTokenTransaction, totals, isSwap } = unsigned.inner
    if (isSwap) {
      console.log('Approve combining token outputs before the batch is sent')
    } else {
      for (const total of totals) {
        console.log(`Approve sending ${total.amount} of token ${total.tokenIdentifier}`)
      }
    }
    signature = new TransferSignature.Token({
      signed: await signer.prepareTokenTransaction(prepareTokenTransaction)
    })
    break
  }
}

const signedPackage = { unsigned, signature }
```

### Flutter

```dart
TransferSignature signature;
if (unsigned is UnsignedTransferPackage_Transfer) {
  // Show the user what they are approving before signing
  final target = unsigned.target;
  String destination = "";
  if (target is TransferTarget_Spark) {
    destination = target.address;
  } else if (target is TransferTarget_Lightning) {
    destination = target.bolt11;
  } else if (target is TransferTarget_CoopExit) {
    destination = target.address;
  }
  print("Approve sending ${unsigned.amountSat} sats"
      " (fee ${unsigned.feeSat} sats) to $destination");
  signature = TransferSignature.transfer(
      signed: await signer.prepareTransfer(unsigned.prepareTransfer));
} else if (unsigned is UnsignedTransferPackage_Swap) {
  print("Approve re-shaping funds for a ${unsigned.amountSat} sat send"
      " (fee ${unsigned.feeSat} sats)");
  signature = TransferSignature.transfer(
      signed: await signer.prepareTransfer(unsigned.prepareTransfer));
} else if (unsigned is UnsignedTransferPackage_Token) {
  if (unsigned.isSwap) {
    print("Approve combining token outputs for a ${unsigned.tokenIdentifier} send");
  } else {
    print("Approve sending ${unsigned.amount} of token"
        " ${unsigned.tokenIdentifier} (fee ${unsigned.fee})");
  }
  signature = TransferSignature.token(
      signed: await signer
          .prepareTokenTransaction(unsigned.prepareTokenTransaction));
} else if (unsigned is UnsignedTransferPackage_TokenBatch) {
  if (unsigned.isSwap) {
    print("Approve combining token outputs before the batch is sent");
  } else {
    for (final total in unsigned.totals) {
      print("Approve sending ${total.amount} of token"
          " ${total.tokenIdentifier}");
    }
  }
  signature = TransferSignature.token(
      signed: await signer
          .prepareTokenTransaction(unsigned.prepareTokenTransaction));
} else {
  throw Exception("Unknown transfer package variant");
}

final signedPackage =
    SignedTransferPackage(unsigned: unsigned, signature: signature);
```

### Python

```python
if isinstance(unsigned, UnsignedTransferPackage.TRANSFER):
    # Show the user what they are approving before signing
    target = unsigned.target
    destination = ""
    if isinstance(target, TransferTarget.SPARK):
        destination = target.address
    elif isinstance(target, TransferTarget.LIGHTNING):
        destination = target.bolt11
    elif isinstance(target, TransferTarget.COOP_EXIT):
        destination = target.address
    logging.debug(
        f"Approve sending {unsigned.amount_sat} sats"
        f" (fee {unsigned.fee_sat} sats) to {destination}"
    )
    signature = TransferSignature.TRANSFER(
        signed=await signer.prepare_transfer(unsigned.prepare_transfer)
    )
elif isinstance(unsigned, UnsignedTransferPackage.SWAP):
    logging.debug(
        f"Approve re-shaping funds for a {unsigned.amount_sat} sat send"
        f" (fee {unsigned.fee_sat} sats)"
    )
    signature = TransferSignature.TRANSFER(
        signed=await signer.prepare_transfer(unsigned.prepare_transfer)
    )
elif isinstance(unsigned, UnsignedTransferPackage.TOKEN):
    if unsigned.is_swap:
        logging.debug(
            f"Approve combining token outputs for a {unsigned.token_identifier} send"
        )
    else:
        logging.debug(
            f"Approve sending {unsigned.amount} of token"
            f" {unsigned.token_identifier} (fee {unsigned.fee})"
        )
    signature = TransferSignature.TOKEN(
        signed=await signer.prepare_token_transaction(
            unsigned.prepare_token_transaction
        )
    )
elif isinstance(unsigned, UnsignedTransferPackage.TOKEN_BATCH):
    if unsigned.is_swap:
        logging.debug("Approve combining token outputs before the batch is sent")
    else:
        for total in unsigned.totals:
            logging.debug(
                f"Approve sending {total.amount} of token {total.token_identifier}"
            )
    signature = TransferSignature.TOKEN(
        signed=await signer.prepare_token_transaction(
            unsigned.prepare_token_transaction
        )
    )
else:
    raise ValueError("Unknown transfer package variant")

signed_package = SignedTransferPackage(unsigned=unsigned, signature=signature)
```

### Go

```go
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
	if pkg.IsSwap {
		log.Printf(
			"Approve combining token outputs for a %v send",
			pkg.TokenIdentifier,
		)
	} else {
		log.Printf(
			"Approve sending %v of token %v (fee %v)",
			pkg.Amount,
			pkg.TokenIdentifier,
			pkg.Fee,
		)
	}
	signed, err := signer.PrepareTokenTransaction(pkg.PrepareTokenTransaction)
	if err != nil {
		return breez_sdk_spark.SignedTransferPackage{}, err
	}
	signature = breez_sdk_spark.TransferSignatureToken{Signed: signed}
case breez_sdk_spark.UnsignedTransferPackageTokenBatch:
	if pkg.IsSwap {
		log.Printf("Approve combining token outputs before the batch is sent")
	} else {
		for _, total := range pkg.Totals {
			// Unset would mean sats, which a batch cannot send yet
			tokenID := ""
			if total.TokenIdentifier != nil {
				tokenID = *total.TokenIdentifier
			}
			log.Printf(
				"Approve sending %v of token %s",
				total.Amount,
				tokenID,
			)
		}
	}
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
```



## Driving the send from the server

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package

Prepare once, then repeat build, sign and publish until the payment is sent:

### Rust

```rust
let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: "<spark address or invoice>".to_string(),
        },
        amount: Some(5_000),
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

loop {
    let unsigned = sdk
        .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
            prepare_response: prepare_response.clone(),
            options: None,
        })
        .await?;

    // Send the package to the user, who reviews and signs it
    let signed_package = sign_package(signer, unsigned).await?;

    match sdk
        .publish_signed_transfer_package(PublishSignedTransferPackageRequest { signed_package })
        .await?
    {
        // The wallet's funds were re-shaped first: build the payment again
        PublishSignedTransferPackageResponse::SwapCompleted => continue,
        PublishSignedTransferPackageResponse::PaymentSent { payment } => {
            return Ok(payment);
        }
        // Only a batch package pays several recipients at once
        PublishSignedTransferPackageResponse::PaymentsSent { .. } => {
            anyhow::bail!("unexpected batch response for a single payment")
        }
    }
}
```

### Swift

```swift
let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: "<spark address or invoice>"),
        amount: BInt(5_000),
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

while true {
    let unsigned = try await sdk.buildUnsignedTransferPackage(
        request: BuildUnsignedTransferPackageRequest(
            prepareResponse: prepareResponse,
            options: nil
        ))

    // Send the package to the user, who reviews and signs it
    let signedPackage = try await signPackage(signer: signer, unsigned: unsigned)

    let publishResponse = try await sdk.publishSignedTransferPackage(
        request: PublishSignedTransferPackageRequest(signedPackage: signedPackage))

    switch publishResponse {
    // The wallet's funds were re-shaped first: build the payment again
    case .swapCompleted:
        continue
    case let .paymentSent(payment):
        return payment
    // Only a batch package pays several recipients at once
    case .paymentsSent:
        throw SdkError.InvalidInput("unexpected batch response for a single payment")
    }
}
```

### Kotlin

```kotlin
val prepareResponse = sdk.prepareSendPayment(
    PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = "<spark address or invoice>"),
        amount = BigInteger.fromLong(5_000L),
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
)

while (true) {
    val unsigned = sdk.buildUnsignedTransferPackage(
        BuildUnsignedTransferPackageRequest(
            prepareResponse = prepareResponse,
            options = null,
        )
    )

    // Send the package to the user, who reviews and signs it
    val signedPackage = signPackage(signer, unsigned)

    val result = sdk.publishSignedTransferPackage(
        PublishSignedTransferPackageRequest(signedPackage)
    )
    when (result) {
        // The wallet's funds were re-shaped first: build the payment again
        is PublishSignedTransferPackageResponse.SwapCompleted -> continue
        is PublishSignedTransferPackageResponse.PaymentSent -> return result.payment
        // Only a batch package pays several recipients at once
        is PublishSignedTransferPackageResponse.PaymentsSent ->
            throw IllegalStateException("unexpected batch response for a single payment")
    }
}
```

### C#

```csharp
var prepareResponse = await sdk.PrepareSendPayment(
    request: new PrepareSendPaymentRequest(
        paymentRequest: new PaymentRequest.Input(input: "<spark address or invoice>"),
        amount: 5_000UL,
        tokenIdentifier: null,
        conversionOptions: null,
        feePolicy: null
    )
);

while (true)
{
    var unsigned = await sdk.BuildUnsignedTransferPackage(
        request: new BuildUnsignedTransferPackageRequest(
            prepareResponse: prepareResponse,
            options: null
        )
    );

    // Send the package to the user, who reviews and signs it
    var signedPackage = await SignPackage(signer, unsigned);

    var response = await sdk.PublishSignedTransferPackage(
        request: new PublishSignedTransferPackageRequest(signedPackage: signedPackage)
    );

    switch (response)
    {
        // The wallet's funds were re-shaped first: build the payment again
        case PublishSignedTransferPackageResponse.SwapCompleted:
            continue;
        case PublishSignedTransferPackageResponse.PaymentSent paymentSent:
            return paymentSent.payment;
        // Only a batch package pays several recipients at once
        case PublishSignedTransferPackageResponse.PaymentsSent:
            throw new Exception("unexpected batch response for a single payment");
    }
}
```

### Javascript (Wasm)

```typescript
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
  // Only a batch package pays several recipients at once
  if (publishResponse.type === 'paymentsSent') {
    throw new Error('unexpected batch response for a single payment')
  }
  return publishResponse.payment
}
```

### React Native

```typescript
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
  // Only a batch package pays several recipients at once
  if (publishResponse.tag === PublishSignedTransferPackageResponse_Tags.PaymentsSent) {
    throw new Error('unexpected batch response for a single payment')
  }
  return publishResponse.inner.payment
}
```

### Flutter

```dart
final prepareResponse = await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest:
            PaymentRequest.input(input: "<spark address or invoice>"),
        amount: BigInt.from(5000),
        tokenIdentifier: null,
        conversionOptions: null,
        feePolicy: null));

while (true) {
  final unsigned = await sdk.buildUnsignedTransferPackage(
      request: BuildUnsignedTransferPackageRequest(
          prepareResponse: prepareResponse, options: null));

  // Send the package to the user, who reviews and signs it
  final signedPackage = await signPackage(signer, unsigned);

  final result = await sdk.publishSignedTransferPackage(
      request:
          PublishSignedTransferPackageRequest(signedPackage: signedPackage));
  if (result is PublishSignedTransferPackageResponse_SwapCompleted) {
    // The wallet's funds were re-shaped first: build the payment again
    continue;
  }
  if (result is PublishSignedTransferPackageResponse_PaymentSent) {
    return result.payment;
  }
  // Only a batch package pays several recipients at once
  if (result is PublishSignedTransferPackageResponse_PaymentsSent) {
    throw Exception("unexpected batch response for a single payment");
  }
}
```

### Python

```python
try:
    prepare_response = await sdk.prepare_send_payment(
        PrepareSendPaymentRequest(
            payment_request=PaymentRequest.INPUT(input="<spark address or invoice>"),
            amount=5_000,
            token_identifier=None,
            conversion_options=None,
            fee_policy=None,
        )
    )

    while True:
        unsigned = await sdk.build_unsigned_transfer_package(
            BuildUnsignedTransferPackageRequest(
                prepare_response=prepare_response, options=None
            )
        )

        # Send the package to the user, who reviews and signs it
        signed_package = await sign_package(signer, unsigned)

        result = await sdk.publish_signed_transfer_package(
            PublishSignedTransferPackageRequest(signed_package=signed_package)
        )
        if isinstance(result, PublishSignedTransferPackageResponse.SWAP_COMPLETED):
            # The wallet's funds were re-shaped first: build the payment again
            continue
        if isinstance(result, PublishSignedTransferPackageResponse.PAYMENT_SENT):
            return result.payment
        # Only a batch package pays several recipients at once
        if isinstance(result, PublishSignedTransferPackageResponse.PAYMENTS_SENT):
            raise ValueError("unexpected batch response for a single payment")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
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
	// Only a batch package pays several recipients at once
	case breez_sdk_spark.PublishSignedTransferPackageResponsePaymentsSent:
		return nil, errors.New("unexpected batch response for a single payment")
	}
}
```



### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

#### Rust

```rust
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
let unsigned = sdk
    .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
        prepare_response,
        options: Some(BuildTransferPackageOptions::BitcoinAddress {
            confirmation_speed: OnchainConfirmationSpeed::Medium,
        }),
    })
    .await?;
```

#### Swift

```swift
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
let unsigned = try await sdk.buildUnsignedTransferPackage(
    request: BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: BuildTransferPackageOptions.bitcoinAddress(
            confirmationSpeed: OnchainConfirmationSpeed.medium
        )
    ))
```

#### Kotlin

```kotlin
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
val unsigned = sdk.buildUnsignedTransferPackage(
    BuildUnsignedTransferPackageRequest(
        prepareResponse = prepareResponse,
        options = BuildTransferPackageOptions.BitcoinAddress(
            confirmationSpeed = OnchainConfirmationSpeed.MEDIUM
        ),
    )
)
```

#### C#

```csharp
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
var unsigned = await sdk.BuildUnsignedTransferPackage(
    request: new BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: new BuildTransferPackageOptions.BitcoinAddress(
            confirmationSpeed: OnchainConfirmationSpeed.Medium
        )
    )
);
```

#### Javascript (Wasm)

```typescript
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
const unsigned = await sdk.buildUnsignedTransferPackage({
  prepareResponse,
  options: {
    type: 'bitcoinAddress',
    confirmationSpeed: 'medium'
  }
})
```

#### React Native

```typescript
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
const unsigned = await sdk.buildUnsignedTransferPackage({
  prepareResponse,
  options: new BuildTransferPackageOptions.BitcoinAddress({
    confirmationSpeed: OnchainConfirmationSpeed.Medium
  })
})
```

#### Flutter

```dart
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
final unsigned = await sdk.buildUnsignedTransferPackage(
    request: BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: BuildTransferPackageOptions.bitcoinAddress(
            confirmationSpeed: OnchainConfirmationSpeed.medium)));
```

#### Python

```python
# For Bitcoin address sends, the confirmation speed is chosen when
# building the package: the fee depends on it
try:
    unsigned = await sdk.build_unsigned_transfer_package(
        BuildUnsignedTransferPackageRequest(
            prepare_response=prepare_response,
            options=BuildTransferPackageOptions.BITCOIN_ADDRESS(
                confirmation_speed=OnchainConfirmationSpeed.MEDIUM
            ),
        )
    )
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
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
```



### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): `prefer_spark` sends via a direct Spark transfer when the invoice also contains a Spark address, and `completion_timeout_secs` controls how long publishing waits for the payment to complete before returning it while still pending:

#### Rust

```rust
let unsigned = sdk
    .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
        prepare_response,
        options: Some(BuildTransferPackageOptions::Bolt11Invoice {
            prefer_spark: true,
            completion_timeout_secs: Some(10),
        }),
    })
    .await?;
```

#### Swift

```swift
let unsigned = try await sdk.buildUnsignedTransferPackage(
    request: BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: BuildTransferPackageOptions.bolt11Invoice(
            preferSpark: true,
            completionTimeoutSecs: 10
        )
    ))
```

#### Kotlin

```kotlin
val unsigned = sdk.buildUnsignedTransferPackage(
    BuildUnsignedTransferPackageRequest(
        prepareResponse = prepareResponse,
        options = BuildTransferPackageOptions.Bolt11Invoice(
            preferSpark = true,
            completionTimeoutSecs = 10u,
        ),
    )
)
```

#### C#

```csharp
var unsigned = await sdk.BuildUnsignedTransferPackage(
    request: new BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: new BuildTransferPackageOptions.Bolt11Invoice(
            preferSpark: true,
            completionTimeoutSecs: 10
        )
    )
);
```

#### Javascript (Wasm)

```typescript
const unsigned = await sdk.buildUnsignedTransferPackage({
  prepareResponse,
  options: {
    type: 'bolt11Invoice',
    preferSpark: true,
    completionTimeoutSecs: 10
  }
})
```

#### React Native

```typescript
const unsigned = await sdk.buildUnsignedTransferPackage({
  prepareResponse,
  options: new BuildTransferPackageOptions.Bolt11Invoice({
    preferSpark: true,
    completionTimeoutSecs: 10
  })
})
```

#### Flutter

```dart
final unsigned = await sdk.buildUnsignedTransferPackage(
    request: BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: BuildTransferPackageOptions.bolt11Invoice(
            preferSpark: true, completionTimeoutSecs: 10)));
```

#### Python

```python
try:
    unsigned = await sdk.build_unsigned_transfer_package(
        BuildUnsignedTransferPackageRequest(
            prepare_response=prepare_response,
            options=BuildTransferPackageOptions.BOLT11_INVOICE(
                prefer_spark=True, completion_timeout_secs=10
            ),
        )
    )
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
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
```



### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with `prepare_token_transaction`. A Token package with `is_swap` set means the wallet first needs to combine token outputs: publishing it returns `PublishSignedTransferPackageResponse::SwapCompleted`, just like the Bitcoin case.

## LNURL-Pay

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with `prepare_lnurl_pay` as in [LNURL-Pay](lnurl_pay.md), then run the same loop with `build_unsigned_lnurl_pay_package` and `publish_signed_lnurl_pay_package`. The result carries the LNURL response, including any success action:

### Rust

```rust
loop {
    let unsigned = sdk
        .build_unsigned_lnurl_pay_package(BuildUnsignedLnurlPayPackageRequest {
            prepare_response: prepare_response.clone(),
        })
        .await?;

    let signed_package = sign_package(signer, unsigned).await?;

    match sdk
        .publish_signed_lnurl_pay_package(PublishSignedLnurlPayPackageRequest {
            signed_package,
        })
        .await?
    {
        PublishSignedLnurlPayResponse::SwapCompleted => continue,
        PublishSignedLnurlPayResponse::PaymentSent { response } => {
            return Ok(response);
        }
    }
}
```

### Swift

```swift
while true {
    let unsigned = try await sdk.buildUnsignedLnurlPayPackage(
        request: BuildUnsignedLnurlPayPackageRequest(
            prepareResponse: prepareResponse
        ))

    let signedPackage = try await signPackage(signer: signer, unsigned: unsigned)

    let publishResponse = try await sdk.publishSignedLnurlPayPackage(
        request: PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage))

    switch publishResponse {
    case .swapCompleted:
        continue
    case let .paymentSent(response):
        return response
    }
}
```

### Kotlin

```kotlin
while (true) {
    val unsigned = sdk.buildUnsignedLnurlPayPackage(
        BuildUnsignedLnurlPayPackageRequest(prepareResponse)
    )

    val signedPackage = signPackage(signer, unsigned)

    val result = sdk.publishSignedLnurlPayPackage(
        PublishSignedLnurlPayPackageRequest(signedPackage)
    )
    when (result) {
        is PublishSignedLnurlPayResponse.SwapCompleted -> continue
        is PublishSignedLnurlPayResponse.PaymentSent -> return result.response
    }
}
```

### C#

```csharp
while (true)
{
    var unsigned = await sdk.BuildUnsignedLnurlPayPackage(
        request: new BuildUnsignedLnurlPayPackageRequest(
            prepareResponse: prepareResponse
        )
    );

    var signedPackage = await SignPackage(signer, unsigned);

    var response = await sdk.PublishSignedLnurlPayPackage(
        request: new PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage)
    );

    switch (response)
    {
        case PublishSignedLnurlPayResponse.SwapCompleted:
            continue;
        case PublishSignedLnurlPayResponse.PaymentSent paymentSent:
            return paymentSent.response;
    }
}
```

### Javascript (Wasm)

```typescript
while (true) {
  const unsigned = await sdk.buildUnsignedLnurlPayPackage({ prepareResponse })

  const signedPackage = await signPackage(signer, unsigned)

  const publishResponse = await sdk.publishSignedLnurlPayPackage({ signedPackage })

  if (publishResponse.type === 'swapCompleted') {
    continue
  }
  return publishResponse.response
}
```

### React Native

```typescript
while (true) {
  const unsigned = await sdk.buildUnsignedLnurlPayPackage({ prepareResponse })

  const signedPackage = await signPackage(signer, unsigned)

  const publishResponse = await sdk.publishSignedLnurlPayPackage({ signedPackage })

  if (publishResponse.tag === PublishSignedLnurlPayResponse_Tags.SwapCompleted) {
    continue
  }
  return publishResponse.inner.response
}
```

### Flutter

```dart
while (true) {
  final unsigned = await sdk.buildUnsignedLnurlPayPackage(
      request:
          BuildUnsignedLnurlPayPackageRequest(prepareResponse: prepareResponse));

  final signedPackage = await signPackage(signer, unsigned);

  final result = await sdk.publishSignedLnurlPayPackage(
      request:
          PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage));
  if (result is PublishSignedLnurlPayResponse_SwapCompleted) {
    continue;
  }
  if (result is PublishSignedLnurlPayResponse_PaymentSent) {
    return result.response;
  }
}
```

### Python

```python
try:
    while True:
        unsigned = await sdk.build_unsigned_lnurl_pay_package(
            BuildUnsignedLnurlPayPackageRequest(prepare_response=prepare_response)
        )

        signed_package = await sign_package(signer, unsigned)

        result = await sdk.publish_signed_lnurl_pay_package(
            PublishSignedLnurlPayPackageRequest(signed_package=signed_package)
        )
        if isinstance(result, PublishSignedLnurlPayResponse.SWAP_COMPLETED):
            continue
        if isinstance(result, PublishSignedLnurlPayResponse.PAYMENT_SENT):
            return result.response
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
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
```



## Failures and retries

- Publishing the same signed package twice returns the same result, so it is safe to retry after a lost response or a network error.
- If publishing fails because the wallet's funds moved or fees changed since the package was built, prepare again and restart the loop with a fresh package.
- Never reuse a signature for a changed payment. Any change to the amount, fee or destination needs a new package, reviewed and signed by the user.

## Remote signers

The signature does not have to come from a device holding the mnemonic. Any `ExternalSparkSigner` implementation can sign the package, including one backed by a remote signing service. With Turnkey, a policy can require the end user to approve the transfer signing while the server runs the rest; see [Using Turnkey](turnkey.md#user-approved-payments).

## Limitations

- Payments with a conversion step (see [Converting tokens](token_conversion.md)) are not supported.
- USDC/USDT cross-chain sends are not supported.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
