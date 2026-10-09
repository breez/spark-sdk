# Recovering funds

Your Spark balance is held in a tree of pre-signed Bitcoin transactions, so its funds can always be moved onto the Bitcoin blockchain. Recovering funds does that, in two situations:

- **Funds that left your balance.** Some of your funds can end up on-chain, where Spark can no longer move them and only an on-chain transaction to an address of yours recovers them.
- **A unilateral exit of your balance.** If the Spark operators stop cooperating with normal [withdrawals](send_payment.md), because they are unreachable or refuse to serve your wallet, you can move your balance on-chain without them.

The balance is split into leaves, and each leaf is recovered in one of two ways, which the SDK picks from the leaf's state:

- **A unilateral exit** takes the leaf on-chain through its own pre-signed transactions. It needs nothing from the operators. Those transactions pay no fee on their own, so you pay their fees from a Bitcoin UTXO of your own, and the exit can take several days because of on-chain timelocks.
- **A cooperative recovery** is for a leaf the Spark operators moved on-chain. Its funds then sit in an output no pre-signed transaction spends, so the operators co-sign one transaction that sends them to your address. The transaction pays its fee out of the leaf's value, so it needs no funding.

## Recoverable funds

Funds that left your balance are not part of `balanceSats`. `getInfo` reports them separately, as `recoverableFundsSats`. A sync adds a leaf's value to that total when it sees the leaf leave your balance. The value leaves the total once the SDK has seen the recovery pay the funds out on-chain: a cooperative recovery in a block, or a unilateral exit's sweep in a block. `prepareRecoverFunds`, `recoverFunds` and `checkRecoverFunds` record that when they find it on-chain, and the next sync leaves the leaf out. The total also counts funds too small to be worth recovering at the fee rate you pick.

A wallet restored on another device has no such record. For each leaf whose recovery may already have finished, the SDK checks the chain during a sync, and takes the leaf out of the total when it finds the recovery in a block. A chain service that fails a request, for example because it limits requests, does not fail the sync: the SDK keeps what it found, and the next sync continues from there. Until that check is complete the leaf stays in the total, so the total can include funds that were already recovered. The SDK does not check a leaf whose funds are too small to send on-chain at the lowest fee rate nodes relay, so such a leaf stays in the total.

The SDK emits `SdkEvent.RecoverableFunds` when a sync finds new recoverable funds, carrying the new total. Funds leaving the total emit nothing, so `getInfo` is where the current total is read.

```dart
GetInfoResponse info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));

if (info.recoverableFundsSats > BigInt.zero) {
  print("${info.recoverableFundsSats} sats can be recovered on-chain");
}
```



To recover them, prepare the recovery with `ExitLeafSelection.RecoverableOnly` (see [Choose the leaves](#choose-the-leaves)). That selection also takes the leaves of a recovery already under way: a unilateral exit you started takes its leaves out of the balance too, and the event fires for them. When you prepare a recovery of those leaves again, you get a second recovery that competes with the first. While one is under way, `checkRecoverFunds` is what follows it, and with `ExitLeafSelection.Specific` you prepare a recovery of the other leaves on their own.

## Before you start

Co-signing a cooperative recovery needs the operators to be reachable. A unilateral exit works without them, and needs two things instead:

- **The exit data has to already be on the device.** Preparing and building an exit read each leaf's pre-signed transactions from local storage, so both work with the operators unreachable. What they cannot do is obtain that data: a leaf can be exited this way only once it has been synced at least once while the operators were reachable. The SDK collects it as funds arrive, in the background where background services run, which you can turn off with [`exitChainAutoFetchEnabled`](./config.md#unilateral-exit-data). `syncWallet` collects regardless of that flag and waits for the pass before returning, so calling a sync yourself runs the collection at a moment of your choosing. Once collected it can be kept outside the SDK's storage, see [Back up the exit data](#back-up-the-exit-data).
- **You pay the fees from your own UTXO.** The pre-signed transactions carry no fee, so each is fee-bumped with a child transaction (CPFP) funded by a Bitcoin UTXO you provide. That UTXO must be **native SegWit** (a witness-program script). P2WPKH and P2TR are handled by the built-in signer; any other witness program (for example a P2WSH multisig) works through the `CpfpFundingKind.Custom` funding kind and a custom signer (see [The signer](#the-signer)). Legacy (non-SegWit) scripts are rejected. The UTXO also has to be **confirmed** before the first package goes out: the children are version 3 (TRUC) transactions, which nodes do not relay while any input other than the transaction they fee-bump is unconfirmed.

Either way, **you broadcast the transactions yourself.** The SDK builds and signs them but never broadcasts them.

## How it works

A recovery is three calls:

1. `prepareRecoverFunds` prepares the recovery and returns a quote: it picks the leaves, says how each one is recovered, and reports the exact fee and how much to fund, without needing any funding UTXOs yet.
2. `recoverFunds` takes that quote, plus your funding UTXOs and a signer when the quote has `funding`, has the operators co-sign the cooperative recoveries, and returns the complete, signed set of transactions to broadcast.
3. `checkRecoverFunds` takes that set, reads it back against the chain, and says what to send next.

The third call works from what the second returned, which the SDK does not keep: see [Keep the response](#keep-the-response). A unilateral exit runs for days, so the third call is made many times: after each broadcast, and whenever you want to know how far along it is.

### A cooperative recovery

A cooperative recovery is one transaction per leaf. It spends the leaf's funds from the on-chain output they sit in and pays them to your address, less its own fee. Nothing comes before it, so it can go out as soon as `recoverFunds` returns it.

### A unilateral exit of a single leaf

To move a leaf on-chain you broadcast the chain of transactions from the tree down to that leaf, then a refund transaction, then a final sweep to your destination address. Because the pre-signed transactions pay no fee on their own, each one is broadcast together with a CPFP child that pays its fee.

With one leaf there is no fan-out: your funding UTXO pays the fees directly. You broadcast the tree transactions top to bottom, each with its CPFP child as a package, then the refund once its timelock matures, then the sweep.

![Single-leaf unilateral exit](/guide/images/unilateral_exit_single_leaf.svg)

The blue transactions come pre-signed and fixed; you cannot change them. The grey CPFP children and the green sweep are built for you from the funding you supply, and are what actually pay the fees and deliver the funds to your address.

### A unilateral exit of multiple leaves

Exiting several leaves at once starts with a **fan-out** transaction that splits a single funding UTXO into one output per branch. Leaves that share ancestors in the tree share those transactions too, so a shared ancestor is broadcast only once. Every branch's refund is then pulled into a single sweep.

![Multi-leaf unilateral exit](/guide/images/unilateral_exit_multi_leaf.svg)

## Choose the leaves

The `selection` you prepare the recovery with says which leaves to recover:

- `ExitLeafSelection.RecoverableOnly` takes the leaves that left your balance, when each is worth more than its own recovery cost.
- `ExitLeafSelection.All` takes every leaf worth more than its own recovery cost, including the ones still in your balance. Its cooperative leaves still need the operators.
- `ExitLeafSelection.Specific` takes the leaves you name, whether or not they are worth it.

Every selection leaves out a leaf whose recovery already finished. The SDK knows that from the recovery or sweep it recorded in a block. While that transaction is fewer than six blocks deep, and whenever you name the leaf, the SDK first checks that the transaction is still in a block, and prepares the leaf again when it is not. Every selection also leaves out a cooperative leaf whose funds cannot pay the fee of their own recovery at your fee rate, and one whose funds the wallet has not found: a lower fee rate, or preparing again later, can bring it back.

`ExitLeafSelection.All` is for an emergency only: the operators are unreachable, or refuse to serve your wallet. It moves the leaves still in your balance out of Spark with a unilateral exit, a multi-step on-chain process that needs your own Bitcoin to pay mining fees and can take several days. While the operators cooperate, a normal [withdrawal](send_payment.md) moves the same funds on-chain cheaper and faster.

### Leaf denominations and exit cost

Every leaf is exited by its own chain of transactions, so it carries its own on-chain fee whatever its value. The more leaves your balance is spread across, and the smaller they are, the more of it goes to fees on the way out, and the more low-value leaves an `ExitLeafSelection.All` exit abandons as uneconomical dust.

How the balance is split into leaves is governed by the SDK's leaf optimization, which balances everyday payment experience against unilateral exit value. More, smaller denominations let payments go out without leaf swaps, while fewer, larger denominations cost less to exit. The default leans toward payment experience, which suits most wallets, since a unilateral exit is a rare last resort. See [Custom leaf optimization](optimize.md) to understand this tradeoff and adjust it if your use case calls for it.

## Prepare the recovery

Call `prepareRecoverFunds` with the target `feeRateSatPerVbyte`, your `destination` address, the `selection`, and the `fundingKind` of UTXO you will pay a unilateral exit's fees with. The funding kind is needed only when the selection holds a unilateral exit with steps left to broadcast. It returns a `PrepareRecoverFundsResponse`, and each of its `leaves` says whether its `method` is `RecoveryMethod.Cooperative` or `RecoveryMethod.Unilateral`.

Its fields tell you how much Bitcoin to gather and how to structure it:

- `recoverableValueSats` is the total value of the selected `leaves`, and `totalFeeSats` is the on-chain fee to recover it, broken down into its components below. Compare them to decide whether the recovery is worth it at the current fee rate.
- `funding` says how much to fund, two ways. `singleUtxoSats` is the simplest: fund **one** UTXO of at least this many satoshis and the SDK fans it out across branches. `perBranch` lets you skip the fan-out (and its `fanoutFeeSats`) by funding **one UTXO per branch**, each of at least the amount in its `PerBranchFunding` entry.

Only a unilateral exit with steps left to broadcast needs funding. `funding` is unset when every leaf is recovered cooperatively, or when only a sweep is left, which pays its fee from the refunds it spends.

`skipped` lists the leaves your `selection` covers that are not in `leaves`, each with its value and a `reason`. `SkippedLeafReason.FeeExceedsValue` means that at this fee rate recovering the leaf costs at least what it holds. `SkippedLeafReason.FundsNotFound` means the SDK read the chain and found no output holding the leaf's funds. `SkippedLeafReason.Unverified` means the SDK could not look up where the leaf's funds are, and it looks again the next time you prepare. `SkippedLeafReason.NotRecoverable` means no recovery can be built for the leaf as it stands, and its `message` says why. A leaf whose recovery already finished is not listed.

Preparing also reads the chain, and `exitChainState` carries back what it found for the unilateral exit: which nodes are already on-chain, which refunds landed, and which of those have been swept. `recoverFunds` builds only the steps still left, so it takes the whole `PrepareRecoverFundsResponse` unchanged. `exitChainState` also shows how far an exit has got.

The fee rate can be zero. That builds transactions that pay no fee, which nodes do not relay: they reach a block only through a miner you hand them to, for example an accelerator service paid separately. The same holds for a cooperative recovery that pays its destination less than the dust limit: the SDK builds it, and nodes do not relay it.

### The fee components, and what arrives

A recovery pays its mining fees from two different places, so `totalFeeSats` comes with the split that says which is which. Both `prepareRecoverFunds` and `recoverFunds` report it.

| Component | Paid by |
|---|---|
| `cooperativeFeeSats` | The value being recovered. Each cooperative leaf has two fees: the one of the transaction the operators broadcast to move it on-chain, which is already paid, and the one of its recovery transaction. Zero when there are none |
| `cpfpFeeSats` | The funding UTXOs, through the CPFP children that fee-bump the tree transactions |
| `fanoutFeeSats` | The funding UTXO, by the fan-out transaction. Zero when there is no fan-out |
| `sweepFeeSats` | The value being recovered, by the final sweep |

The four add up to `totalFeeSats`.

The CPFP and fan-out fees come out of the Bitcoin you supplied as funding and do not reduce what the recovery returns. The other two come off the money on its way to your address: the sweep pays out what is left of the refunds after its own fee, and a cooperative recovery pays out what its leaf has left on-chain after its own fee.

**What arrives at `destination`** is therefore `recoverableValueSats` less `cooperativeFeeSats` and `sweepFeeSats`, plus any funding that was not spent on fees. The sweep also collects the leftover change of the CPFP children it built, so unused funding is delivered to the same address rather than left behind. `recoverableValueSats` less `totalFeeSats` is not the arriving amount: it subtracts the CPFP and fan-out fees, which the funding already paid.

**What the recovery costs in total** is `totalFeeSats`, across the funding UTXO and the recovered value together. Beginning with `recoverableValueSats` in Spark and a funding UTXO worth F, the destination ends up with those two added together, less `totalFeeSats`.

`singleUtxoSats` sits above `cpfpFeeSats` plus `fanoutFeeSats` on purpose. It carries the sweep fee and a small per-branch allowance as headroom, and both come back to you in the sweep.

### Worth recovering

Under `ExitLeafSelection.All` and `ExitLeafSelection.RecoverableOnly` a leaf is kept only when its value exceeds its own recovery cost, measured per leaf. That per-leaf measure does not include the shared `fanoutFeeSats`, which the single-UTXO path pays once for the whole exit. So when you fund a multi-leaf exit from a **single** UTXO, the fan-out fee can push the total above what you recover, even though every leaf looked profitable on its own.

Two rules keep a recovery from ever costing more than it returns:

1. **Before funding, require `recoverableValueSats` to exceed `totalFeeSats`.** These are the actual totals for the quote, fan-out fee included. If the margin is thin or negative, do not proceed as prepared.
2. **Prefer per-branch funding.** Funding one UTXO per branch (`perBranch`) skips the fan-out entirely, so there is no shared fee. Because those two selections already keep only leaves worth more than their own cost, such a recovery is always net-positive when funded per branch.

If the single-UTXO total is not worth it, either fund per branch, or narrow the set: prepare again with `ExitLeafSelection.Specific` naming only the higher-value leaves (dropping the marginal ones removes their cost and can turn the total positive), or wait for a lower fee rate.

If nothing is selected (no leaf is worth recovering at the given fee rate, the leaves' recoveries already finished, or there is nothing to recover) the response comes back with no `leaves` rather than as an error. `skipped` names each leaf the SDK left out of the quote and why.

```dart
PrepareRecoverFundsRequest request = PrepareRecoverFundsRequest(
  feeRateSatPerVbyte: BigInt.from(2),
  fundingKind: const CpfpFundingKind.p2Wpkh(),
  destination: "bc1q...your-destination-address",
  selection: const ExitLeafSelection.recoverableOnly(),
);

PrepareRecoverFundsResponse quote = await sdk.prepareRecoverFunds(request: request);

if (quote.leaves.isEmpty) {
  print("Nothing to recover");
  return quote;
}
for (RecoverFundsLeaf leaf in quote.leaves) {
  print("${leaf.leafId}: ${leaf.valueSats} sats, ${leaf.method}");
}
print("Recovering ${quote.recoverableValueSats} sats"
    " for ${quote.totalFeeSats} sats in fees");
RecoveryFunding? funding = quote.funding;
if (funding != null) {
  print("Fund one UTXO of at least ${funding.singleUtxoSats} sats");
}
```



## Build the recovery

Gather funding that meets the quote's `funding`, then call `recoverFunds` with the quote, your `CpfpInput` funding UTXOs, and a signer. It has the operators co-sign each cooperative recovery, signs the unilateral exit, and returns a `RecoverFundsResponse` with the full transaction set and what it actually costs.

The operators co-sign each cooperative recovery on its own. One that is not produced does not fail the call: it is listed in `failed` with the `error` that stopped it and the output it would have spent (`outputTxid` and `outputVout`), and it is left out of `leaves`, `transactions` and the totals. The [troubleshooting](#troubleshooting) table says what each error asks of you.

The SDK keeps every cooperative recovery the operators co-sign. Building a recovery of the same leaf again, to the same destination at the same fee rate, returns the kept transaction instead of asking the operators again, so that works while they are unreachable too. Any cooperative recovery, kept or new, has to pay enough to replace a different recovery of the same leaf that is already on the network. One that does not comes back in `failed` with `CooperativeRecoveryError.ReplacementFeeTooLow`, naming the fee and the fee rate that would. When the chain cannot be read, the SDK takes the highest-fee recovery it kept for the leaf to be the one on the network.

If the funding is below what the exit needs, it returns `SdkError.InsufficientCpfpFunds`, naming the amount. A UTXO an earlier attempt already spent is not an error: see [Funding a second attempt](#funding-a-second-attempt).

A very thin-margin exit can fail even when the funding is sufficient: if the recoverable value net of fees would leave the swept output below the destination address's dust limit, the sweep cannot be built and the exit fails. Exit higher-value leaves with `ExitLeafSelection.Specific`, lower the `feeRateSatPerVbyte`, or wait for a cheaper fee rate.

The set it builds depends on what is already on-chain. Because each CPFP child spends the previous one, the exit is one connected chain, so to continue it correctly the SDK reads confirmed on-chain state through its chain service: a step already confirmed comes back as `ExitTransactionStatus.Confirmed` and is not rebuilt. If the chain service cannot resolve a step, the SDK falls back to the status the operators reported: a node the operators already consider on-chain is left as-is rather than fee-bumped (bumping an already-confirmed node would invalidate the rest of the chain), and any node whose state still cannot be determined comes back as `ExitTransactionStatus.Unverified` rather than failing the build. For a more reliable source you can supply your own chain service (see [Customizing the SDK](customizing.md#with-chain-service)).

```dart
List<int> secretKeyBytes = hex.decode("your-secret-key-hex");

RecoverFundsResponse response = await sdk.recoverFunds(
  request: RecoverFundsRequest(
    prepared: quote,
    fundingInputs: [
      CpfpInput.p2Wpkh(
        txid: "your-utxo-txid",
        vout: 0,
        valueSats: BigInt.from(50000),
        pubkey: "your-compressed-pubkey-hex",
      ),
    ],
  ),
  signerSecretKey: Uint8List.fromList(secretKeyBytes),
);

// Keep the whole response: checkRecoverFunds follows the recovery from it.
for (RecoveryTransaction tx in response.transactions) {
  if (tx.csvTimelockBlocks != null) {
    print("${tx.txid}: wait ${tx.csvTimelockBlocks} blocks after its parents confirm");
  }
}
```



### Recovering without funding

When the quote has no `funding`, there is nothing to fund and nothing for a signer to sign: pass no funding inputs and no signer. When it has, and you want the cooperative leaves recovered without funding the rest, prepare again naming only the cooperative leaves with `ExitLeafSelection.Specific`. That builds a recovery of its own, and the unilateral leaves are left for another.

```dart
// A quote with funding holds a unilateral exit: prepare the
// cooperative leaves alone to recover them without it.
if (quote.funding != null) {
  List<String> leafIds = quote.leaves
      .where((leaf) => leaf.method == RecoveryMethod.cooperative)
      .map((leaf) => leaf.leafId)
      .toList();
  if (leafIds.isEmpty) {
    return;
  }
  quote = await sdk.prepareRecoverFunds(
    request: PrepareRecoverFundsRequest(
      feeRateSatPerVbyte: quote.feeRateSatPerVbyte,
      fundingKind: null,
      destination: quote.destination,
      selection: ExitLeafSelection.specific(leafIds: leafIds),
    ),
  );
}
RecoverFundsResponse response = await sdk.recoverFunds(
  request: RecoverFundsRequest(prepared: quote, fundingInputs: []),
  signerSecretKey: null,
);

// Keep the whole response: checkRecoverFunds follows the recovery from it.
for (RecoveryTransaction tx in response.transactions) {
  print("Broadcast ${tx.txid}: ${tx.txHex}");
}
for (CooperativeRecoveryFailure failure in response.failed) {
  print("Leaf ${failure.leafId} was not recovered: ${failure.error}");
}
```



### The signer

The CPFP children and the fan-out spend your funding UTXOs, so they have to be signed. The SDK does not hold your funding keys; it hands each unsigned transaction to a signer you provide.

The built-in single-key signer covers the common case: it signs P2WPKH and P2TR inputs from one secret key. For `CpfpInput.P2tr` funding, pass the **internal, untweaked (BIP86)** key, not the tweaked on-chain output key: the tweaked key derives a scriptPubKey that does not match the UTXO, so the transaction is rejected at broadcast. For anything else (a multisig, a hardware wallet, or keeping key material out of the SDK entirely) implement the `CpfpSigner` interface and describe the funding with `CpfpFundingKind.Custom` (when preparing) and `CpfpInput.Custom` (when building). Those carry the funding `scriptPubkeyHex` and an upper-bound `signedInputWeight` so the fee stays exact for any witness program. The signer receives a serialized PSBT, signs the inputs that are not already finalized, and returns the serialized signed PSBT:

Whichever signer you use, the funding inputs must be **native SegWit** (a witness-program script; P2WPKH or P2TR with the built-in signer, any other witness program with a custom one). The exit refers to each transaction by an id it computes before signing, which only stays stable when the signature lives in the witness (native SegWit) rather than in the input script; legacy scripts are rejected, so your signer only ever has to sign native SegWit inputs.

```dart
Future<RecoverFundsResponse> recoverWithFundingSigner(
  BreezSdk sdk,
  PrepareRecoverFundsResponse quote,
) async {
  RecoverFundsResponse response = await sdk.recoverFundsWithSigner(
    request: RecoverFundsRequest(
      prepared: quote,
      fundingInputs: [
        CpfpInput.p2Wpkh(
          txid: "your-utxo-txid",
          vout: 0,
          valueSats: BigInt.from(50000),
          pubkey: "your-compressed-pubkey-hex",
        ),
      ],
    ),
    signPsbt: (Uint8List psbtBytes) async {
      Uint8List signedPsbtBytes = await signWithFundingKeys(psbtBytes);
      return signedPsbtBytes;
    },
  );

  return response;
}

Future<Uint8List> signWithFundingKeys(Uint8List psbtBytes) async {
  return psbtBytes;
}
```



**Flutter**

Flutter cannot pass a foreign <code>CpfpSigner</code>, so it exposes two recovery calls. <code>recoverFunds</code> takes the funding secret key bytes, or none when there is nothing to fund, and uses the built-in single-key signer. <code>recoverFundsWithSigner</code> takes a <code>signPsbt</code> callback that receives the serialized PSBT, signs the inputs that are not already finalized (any scheme), and returns the serialized signed PSBT.

## Keep the response

A `RecoverFundsResponse` is the record of the recovery it holds: the signed transactions, the leaves they recover, and the funding they spend. The SDK does not keep it, and `checkRecoverFunds` needs it to follow the recovery. Without it the recovery cannot be followed or finished as built, though the money stays recoverable: when you prepare and build again, the SDK picks it up from wherever it is.

`checkRecoverFunds` returns the same recovery with its statuses brought up to date, to be kept in place of the one passed in. Each call to `recoverFunds` returns a separate recovery, followed on its own: for example the cooperative leaves built without the rest, a failed leaf built again, or a new build at a higher fee rate, which replaces the one before it.

## The transaction set

Each `RecoveryTransaction` in `transactions` carries:

- `kind`: whether it is a cooperative recovery, the fan-out, a tree node, a refund, or the sweep.
- `nodeId`: the tree node a transaction belongs to (the leaf id for a cooperative recovery and a refund), unset for the fan-out and the sweep.
- `txid` and `txHex`: the signed transaction to broadcast.
- `cpfpTxHex`: its signed CPFP child, to broadcast alongside `txHex` as a package. Unset for a cooperative recovery, the fan-out and the sweep, and for a step that is already confirmed.
- `csvTimelockBlocks`: the relative timelock, in blocks, that must mature before the transaction can confirm.
- `dependsOn`: the txids of other transactions in the set that must confirm first.
- `status`: where the transaction stands. `ExitTransactionStatus.Confirmed` means it is done and can be skipped, and carries the `blockHeight` it landed at, which is what a `csvTimelockBlocks` on its child counts from. `ExitTransactionStatus.Ready` means broadcast it now. `ExitTransactionStatus.WaitingForDependencies` means something in `dependsOn` has yet to confirm. `ExitTransactionStatus.WaitingForTimelock` means its inputs are confirmed but its `csvTimelockBlocks` has not matured, and reports the `spendableAtHeight` block it can first be mined in. `ExitTransactionStatus.Unverified` means the chain could not be read for it while the recovery was built, so whether it is waiting, ready or already confirmed is unknown. A cooperative recovery has this status when the SDK did not see the output it spends in a block, or could not check whether an earlier recovery already spent that output. It can still be broadcast: if it, or a transaction spending the same funds, is already in a block, the network rejects it.

## Broadcast the transactions

`transactions` is the complete, signed set in valid broadcast order, and it is yours to send to the network over time. Broadcast each transaction whose `status` is `ExitTransactionStatus.Ready`, and leave the rest until a later `checkRecoverFunds` reports them ready. Because of the timelocks in the tree, a unilateral exit can span several days.

A cooperative recovery is ready as soon as it is returned. Its funds stay in `recoverableFundsSats` until the SDK on this device has found the recovery in a block, as [Recoverable funds](#recoverable-funds) describes. The SDK looks for that in `prepareRecoverFunds`, `recoverFunds` and `checkRecoverFunds`, and during syncs at most once per leaf. Another device keeps the funds in its own total until the SDK there has found the recovery.

**A step left waiting can be sent by the operators**

Each step of a unilateral exit becomes valid at a certain block. About 50 blocks later, which is roughly eight hours, a second version of that same step becomes valid too. The Spark operators hold that second version in their watchtowers, as a safety net for a wallet that goes offline part-way through an exit, and can send it to the network once it unlocks.

The version the SDK builds for you is paid for by the funding UTXO you supplied, at the fee rate you asked for. The operators' version has its fee built in and takes it from the leaf itself, so that fee comes off the amount arriving at your address, at a rate you have no say in. At some steps the operators' version also moves the leaf's funds to an output the leaf's pre-signed transactions do not spend. The leaf then leaves your exit, and only a cooperative recovery, with the operators, reaches it. For some leaves the operators cannot co-sign that either, and their funds stay out of reach.

The window runs per step, from the moment that step's timelock matures. An exit whose steps go out as they become `ExitTransactionStatus.Ready` keeps the fee split of the quote, and keeps all its leaves.

### Broadcast each package together

Most steps of a unilateral exit come as a pair: a tree transaction and its `cpfpTxHex` CPFP child. The tree transaction pays no fee on its own, so a normal single-transaction broadcast rejects it; only the child makes the pair pay enough. Broadcast the two together, as a package, with a node that supports package relay, for example Bitcoin Core:

```text
bitcoin-cli submitpackage '["<tx_hex>", "<cpfp_tx_hex>"]'
```

The **fan-out**, the **sweep** and a **cooperative recovery** are the exceptions: each pays its own fee and has no CPFP child (`cpfpTxHex` is unset), so each goes out **alone**, as an ordinary transaction, through any node or a public endpoint such as `POST https://mempool.space/api/tx`. At a zero fee rate they too have to reach a miner directly (see [Prepare the recovery](#prepare-the-recovery)). Most public broadcast APIs, including mempool.space, accept only one transaction at a time and cannot submit a package, so they reject the zero-fee tree transactions; use a package-relay-capable node (or service) for the pairs.

### Wait for each step to confirm

Within a branch you broadcast one package, wait for it to confirm, then broadcast the next. This is a mempool relay limit, not a Bitcoin consensus rule: nodes relay an unconfirmed parent with at most one unconfirmed child (the "one-parent-one-child", or 1P1C, package), so a second still-unconfirmed package stacked on top would not propagate. Once a package confirms, the next one has a confirmed parent and can go out. (A refund's `csvTimelockBlocks` is a separate wait, and that one is a consensus rule.)

### Order and parallelism

Follow `dependsOn` to order the set: a transaction can go out as soon as the transactions it lists have confirmed. Cooperative recoveries list nothing, and a single leaf's exit is one straight line, top to bottom. With several leaves the branches are largely independent, so to finish faster you can broadcast them in parallel and serialize only where `dependsOn` actually links them:

1. **The fan-out first, and alone.** It pays its own fee and has no CPFP child, so it is an ordinary single-transaction broadcast. Wait for it to confirm before any branch package: every branch's first package depends on it.
2. **Then the branch packages, each node transaction with its CPFP child.** A shared ancestor appears once, listed in the `dependsOn` of every branch that needs it, so you broadcast it a single time. Within a branch, send one package, wait for it to confirm, then the next (the 1P1C limit above); across branches you can work in parallel.
3. **The sweep last, and alone,** once every refund in its `dependsOn` has confirmed.

## Follow the recovery

`checkRecoverFunds` takes a recovery you kept and returns it with every transaction's status brought up to date. It reads the chain and nothing else: no wallet, no leaves, no signer, no funding. A recovery can be followed on a device that has lost everything but the kept response. When `checkRecoverFunds` finds a leaf's recovery in a block, it records that, so the next sync takes the leaf out of `recoverableFundsSats`.

Its `verdict` says what to do next:

- `RecoveryVerdict.Valid`: the recovery is on track. Broadcast the transactions whose `status` is `ExitTransactionStatus.Ready`. Sending one you already sent is harmless, so you never have to remember what you broadcast.
- `RecoveryVerdict.Done`: every transaction has confirmed, and there is nothing left to do. A cooperative recovery counts as confirmed once any recovery of the same leaf is, so one you replaced with a higher fee reads as confirmed when its replacement confirms. The leaves in `failed` are not part of the recovery: they need one of their own.
- `RecoveryVerdict.Redo`: this recovery cannot finish as it stands. See [Starting over](#starting-over).

`RecoveryVerdict.Redo` carries a `reason`. `RecoveryRedoReason.OnChainStateDiverged` means something on-chain no longer matches the transactions you hold: the Spark operators took a leaf on-chain before your exit reached it, a different refund for the same leaf confirmed, someone fee-bumped a step in a way yours cannot follow, or funding you were counting on went elsewhere. When you prepare and build again, the SDK picks each leaf up from wherever it is. A leaf the operators took on-chain comes back as `RecoveryMethod.Cooperative` once the wallet has found its funds on-chain.

A recovery that an earlier SDK version returned may not parse as a `RecoverFundsResponse` of this version. `checkRecoverFunds` then fails with an error. Keep the recovery you stored, because it names the leaves and the funding inputs, and prepare and build again with those.

```dart
CheckRecoverFundsResponse checked = await sdk.checkRecoverFunds(
  request: CheckRecoverFundsRequest(recovery: stored),
);

// Store this one in place of the one you had.
RecoverFundsResponse recovery = checked.recovery;

RecoveryVerdict verdict = checked.verdict;
if (verdict is RecoveryVerdict_Valid) {
  for (RecoveryTransaction tx in recovery.transactions) {
    if (tx.status is ExitTransactionStatus_Ready) {
      print("ready to broadcast: ${tx.txid}");
    }
  }
} else if (verdict is RecoveryVerdict_Done) {
  print("Every transaction confirmed: the recovery is done");
} else if (verdict is RecoveryVerdict_Redo) {
  // Prepare and build again, naming the same leaves. Pass
  // recovery.fundingInputs back and the SDK follows them to whatever they
  // have become.
  print("Build the recovery again: ${verdict.reason}");
}
```



## Funding a second attempt

Whenever you call `recoverFunds` again for a unilateral exit, you have to give it funding. Two options, and the first is simpler:

- **Fresh UTXOs.** Fund the amount in the new quote and pass those. Nothing to keep track of.
- **The same UTXOs as last time.** Pass back the `fundingInputs` the kept response carries. An earlier attempt will have spent them, and that is fine: the SDK follows each outpoint to what your money became, whether that is a fan-out output, the change of a fee-bumping transaction, or several steps of both. Only what came from money you supplied, and still pays a script you control, is used.

Either way you can add more: pass the old funding *and* a fresh UTXO when the exit needs more than what is left.

The exit is short only when what you gave it, once followed, cannot cover what remains. Then it returns `SdkError.InsufficientCpfpFunds` with the amount it needs.

## Starting over

Each of the following sends you back to `prepareRecoverFunds` and `recoverFunds`. In every case you build the recovery again from scratch: you never hand a previously built transaction back to the SDK.

**You want to pay a higher fee rate.** On-chain fees rise, and a recovery already under way stops confirming. Prepare again at the higher `feeRateSatPerVbyte`, naming the same leaves with `ExitLeafSelection.Specific`, and build again. Whatever has already confirmed stays as it is and costs nothing to keep; only what has not yet confirmed is rebuilt at the higher rate, and it replaces the earlier version on the network (RBF). The fee in the new quote is for the part that is left, so it is less than a fresh exit of the same leaves. The operators co-sign a new cooperative recovery the same way, and it has to pay enough to replace the earlier one if that is already on the network (see [Build the recovery](#build-the-recovery)).

**`checkRecoverFunds` returned `RecoveryVerdict.Redo`.** The chain no longer matches the recovery you hold, so those transactions cannot finish. Prepare and build again the same way: the new recovery picks the funds up from wherever they are, in the tree or on-chain.

**An `ExitTransactionStatus.Unverified` transaction was rejected.** The chain could not be read for it while the recovery was built, so an earlier fee-bumping child may already have spent the funding it uses. `checkRecoverFunds` cannot settle that: it reads only the recovery you kept, never your funding. Building again does, because it follows your funding to what it is worth now, so prepare and build again once the chain service is healthy. For a cooperative recovery `checkRecoverFunds` does settle it: the SDK reports the recovery as `ExitTransactionStatus.Ready` once the chain service shows the output it spends in a block.

**A leaf is in `failed`.** Once its error is dealt with (see [Troubleshooting](#troubleshooting)), prepare its recovery again with `ExitLeafSelection.Specific` and build. That makes a separate recovery, followed on its own.

Name the leaves with `ExitLeafSelection.Specific` rather than `ExitLeafSelection.All` or `ExitLeafSelection.RecoverableOnly`, taking the ids from the kept response. This is the dependable way to pick a recovery back up, including a leaf still waiting out its refund timelock.

Both calls read the chain, so both price only what is left. A leaf far enough along stays worth exiting under `ExitLeafSelection.All` even when a fresh exit of it would not be. A leaf whose refund was already swept has nothing left to exit, so every selection leaves it out, and `exitChainState` shows its refund swept. A cooperative leaf is left out the same way once the wallet has seen its recovery confirm.

## Back up the exit data

The transactions a unilateral exit is built from are held in the SDK's local storage. While the operators are reachable they can be fetched again, so a wallet restored from its seed rebuilds them on its own. When that storage is gone and the operators are unreachable, they cannot be recovered from anywhere, and the leaves they cover cannot be exited.

`exportUnilateralExitState` returns that data as a single opaque value, covering every leaf the wallet holds together with the transactions that spend it. It reflects what is present when it is called: a leaf whose data has not been collected yet is exported without it. The value grows with the number of leaves and can reach several megabytes.

Treat the value as sensitive. Carrying every leaf and its transactions, it discloses the wallet's balance, how that balance is split up, and the history of what the wallet has received and spent. Encrypt it wherever you keep it.

```dart
ExportUnilateralExitStateResponse exported = await sdk.exportUnilateralExitState();

// Keep the state somewhere the wallet's own storage cannot take with it.
print("Exit state is ${exported.exitState.length} bytes");
```



The SDK emits `SdkEvent.UnilateralExitStateChanged` once it has completed the data for a leaf that was missing it, and whenever it rebuilds a leaf's data. That is the point at which a previously exported value stops covering the wallet. A leaf the operators answer for only in part is not announced: what came back still cannot back an exit, and it stays that way until they complete it.

`importUnilateralExitState` puts an exported value back. It does not contact the operators, so it works while they are unreachable, and the value must come from the same network the SDK is configured for. A leaf is taken only when the exit state records this wallet as its owner; the rest are skipped and counted in `skippedForeignLeaves`.

For the leaves it does take, the wallet keeps whatever exit data it can already exit with. An exported value carries no mark of when it was taken, so nothing in it says it is newer than what is on the device; the imported copy is used only for a leaf the wallet has nothing usable for, and only when that copy is complete on its own. Importing an out of date or half-collected value therefore never leaves a leaf less exitable than it already was. Leaves the wallet keeps but whose imported copy it did not use are counted in `skippedChains`.

A leaf is dropped outright when its imported copy disagrees with a node the wallet already holds, on a value that cannot change over a node's lifetime. One of the two copies is then simply wrong about that node, and nothing in the entry is trusted on the strength of it, so the leaf is not restored at all. These are counted separately, in `skippedConflictingLeaves`, because unlike the counts above they mark exit data the import could not put back.

```dart
ImportUnilateralExitStateResponse imported = await sdk.importUnilateralExitState(
  request: ImportUnilateralExitStateRequest(exitState: exitState),
);

print("Imported ${imported.importedLeaves} leaves,"
    " skipped ${imported.skippedForeignLeaves}");
```



An out of date value can restore leaves that have since been spent, so the balance may read high until the next sync reconciles it with the operators.

## Troubleshooting

| Problem | Cause | Solution |
|---------|-------|----------|
| `prepareRecoverFunds` returns no `leaves` | No leaf is worth recovering at the current rate, the leaves' recoveries already finished, or there is nothing to recover | Lower `feeRateSatPerVbyte` or wait for cheaper on-chain fees. A finished recovery has nothing left to recover (this is not an error) |
| A leaf your `selection` covers is missing from the `leaves` of the quote | Its recovery finished, or the SDK left it out of the quote | A leaf the SDK left out is in `skipped` with its `reason`. Lower `feeRateSatPerVbyte` for `SkippedLeafReason.FeeExceedsValue`, and prepare again later for `SkippedLeafReason.Unverified` |
| A leaf you are mid-recovery on is missing from a new `ExitLeafSelection.All` or `ExitLeafSelection.RecoverableOnly` quote | Those selections keep only leaves worth recovering at the new fee rate | Prepare the recovery with `ExitLeafSelection.Specific`, naming the leaves from the kept response |
| A leaf is in `failed` with `CooperativeRecoveryError.OperatorsUnavailable` | The operators could not be reached to co-sign its recovery | Prepare and build again once they are reachable |
| A leaf is in `failed` with `CooperativeRecoveryError.ReplacementFeeTooLow` | An earlier recovery of it is on the network, and the new one pays too little to replace it | Prepare and build again at a fee rate of at least the one the error names, or let the earlier recovery confirm |
| A leaf is in `failed` with `CooperativeRecoveryError.Generic` | The operators refused to co-sign, the wallet's signer could not take part, the chain service failed, or the chain shows the leaf's funds in another output than the SDK assumed | The message says why. A wallet signing with Turnkey cannot recover cooperatively yet (see [Using Turnkey](turnkey.md#availability)) |
| `checkRecoverFunds` returns `RecoveryVerdict.Redo` | Something on-chain no longer matches the transactions you hold, for example the Spark operators took a leaf on-chain | Prepare and build again, naming the same leaves; see [Starting over](#starting-over) |
| The recovery has stopped confirming | On-chain fees rose above what its transactions pay | Prepare and build again at a higher `feeRateSatPerVbyte`; see [Starting over](#starting-over) |
| Less arrived than `recoverableValueSats` less `cooperativeFeeSats` and `sweepFeeSats` | A step sat unbroadcast long enough for the operators to send their own version, which pays its fee out of the leaf | Broadcast each step while it is `ExitTransactionStatus.Ready`; see [A step left waiting can be sent by the operators](#broadcast-the-transactions) |
| `totalFeeSats` is close to or above `recoverableValueSats` | The shared fan-out fee makes a single-UTXO multi-leaf exit uneconomical | Fund one UTXO per branch (`perBranch`) to drop the fan-out fee, recover fewer leaves with `ExitLeafSelection.Specific`, or wait for a lower fee rate |
| The build/sweep fails with a "below the dust limit" error | The recoverable value net of fees is below the destination's dust limit | Exit higher-value leaves with `ExitLeafSelection.Specific`, lower the `feeRateSatPerVbyte`, or wait for a cheaper fee rate |
| `SdkError.InsufficientCpfpFunds` | The funding you gave, once followed to what it became, is below what the exit needs | Fund at least `singleUtxoSats`, or the amount in each `PerBranchFunding`; you can pass fresh UTXOs alongside the old ones |
| `SdkError.InvalidInput` about the destination | The address is malformed, or belongs to another network | Use an address of the network the SDK is configured for |
| `SdkError.InvalidInput`: a leaf has no funds this wallet can recover cooperatively | The quote lists a cooperative leaf this wallet has no record of, for example a quote made by another wallet | Prepare again with this wallet |
| `SdkError.InvalidInput` asking for a funding kind | The selection holds a unilateral exit with steps left to broadcast | Prepare the recovery with a `fundingKind` |
| `SdkError.InvalidInput` asking for a signer or a funding input | The quote's `funding` is set, and the funding inputs or the signer to sign them are missing | Pass funding and a signer, or prepare a recovery of the cooperative leaves alone; see [Recovering without funding](#recovering-without-funding) |
| `SdkError.InvalidInput` about a funding input | A funding UTXO is malformed, or a custom one does not pay a native SegWit script | Check each `CpfpInput` against the UTXO it describes |
| `SdkError.InvalidInput` from `checkRecoverFunds` | A transaction in the recovery passed in does not parse | Pass the recovery as `recoverFunds` or `checkRecoverFunds` returned it |
| "min relay fee not met" when broadcasting | The fee rate is below the network's minimum relay fee rate, zero included | Build again at a higher `feeRateSatPerVbyte`, or hand the transactions to a miner directly |
| "TRUC-violation" when broadcasting a package | The funding UTXO is not confirmed yet | Wait for the transaction that created it to confirm, then broadcast again |
| "mandatory-script-verify-flag-failed" | A CPFP child was not signed correctly | Ensure your `CpfpSigner` signs every non-finalized input |
| "non-BIP68-final" | A relative timelock has not matured | Wait until `status` leaves `ExitTransactionStatus.WaitingForTimelock` |
| A tree transaction is rejected on its own | The zero-fee parent was broadcast without its child | Broadcast the parent and its `cpfpTxHex` together as a package |
| The sweep is rejected | Not every refund it spends has confirmed | Wait until `checkRecoverFunds` reports it `ExitTransactionStatus.Ready` |
| An `ExitTransactionStatus.Unverified` transaction is rejected | The chain service was unavailable or rate-limited while the recovery was built, so the SDK could not tell whether that step is already on-chain, nor whether an earlier fee-bumping child already spent the funding it uses | Prepare and build the recovery again once the chain service is healthy; see [Starting over](#starting-over), and [Customizing the SDK](customizing.md#with-chain-service) for a more reliable service |
