import breez_sdk_spark.BreezSdk
import breez_sdk_spark.GetInfoRequest
import breez_sdk_spark.PrepareSendPaymentRequest
import breez_sdk_spark.ReceivePaymentMethod
import breez_sdk_spark.ReceivePaymentRequest
import breez_sdk_spark.Seed
import breez_sdk_spark.SendPaymentRequest

import com.ionspin.kotlin.bignum.integer.BigInteger

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

// --- fund mode (treasurer top-up via Lightspark regtest faucet) -----------

/**
 * Idempotent treasurer top-up: walks the treasurer's balance up to
 * `targetSats` by repeatedly hitting the faucet (capped at
 * [Faucet.MAX_PER_CALL_SATS] per call) and waiting for each on-chain
 * deposit to be claimed before requesting the next chunk.
 *
 * If the treasurer is already at or above `targetSats`, exits without
 * calling the faucet — safe to re-run.
 */
fun fundTreasurer(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val targetSats = opts["target-sats"]?.toLongOrNull() ?: 5_000_000L

    System.getenv("FAUCET_USERNAME") ?: error("FAUCET_USERNAME env var is required")
    System.getenv("FAUCET_PASSWORD") ?: error("FAUCET_PASSWORD env var is required")

    println("[fund] treasurer top-up to $targetSats sats  mysql=${maskPassword(mysqlUrl)}")

    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, TREASURER_USER_ID))
    val handlers = SharedHandlers.create(mysqlUrl)
    val sdk = buildSdk(benchConfig(), seed, handlers)

    try {
        // Skip sync — local balance is a lower bound, sufficient for ≥-target check.
        val cachedBalance = sdk.getInfo(GetInfoRequest(ensureSynced = false)).balanceSats.toLong()
        if (cachedBalance >= targetSats) {
            println("[fund] cached balance: $cachedBalance sats (≥ $targetSats target, skipping sync)")
            println("[fund] OK")
            return@runBlocking
        }
        println("[fund] cached balance: $cachedBalance sats (below $targetSats; syncing to confirm)")

        // Reuse an existing deposit address if the treasurer has one.
        val depositAddr = sdk.receivePayment(
            ReceivePaymentRequest(paymentMethod = ReceivePaymentMethod.BitcoinAddress(newAddress = false))
        ).paymentRequest
        println("[fund] deposit address: $depositAddr")

        var chunkIdx = 0
        while (true) {
            val info = sdk.syncedInfo()
            val balance = info.balanceSats.toLong()
            if (balance >= targetSats) {
                println("[fund] treasurer balance: $balance sats (target reached)")
                break
            }
            val remaining = targetSats - balance
            var chunk = remaining.coerceAtMost(Faucet.MAX_PER_CALL_SATS)
            if (chunk < Faucet.MIN_PER_CALL_SATS) chunk = Faucet.MIN_PER_CALL_SATS
            chunkIdx++
            println("[fund] chunk #$chunkIdx: requesting $chunk sats (balance $balance/$targetSats)")
            val txid = Faucet.fundBitcoinAddress(depositAddr, chunk)
            println("[fund] chunk #$chunkIdx faucet txid: $txid")
            waitForBalanceIncrease(
                sdk,
                balance.toULong(),
                timeoutMs = 240_000,
                pollLabel = "[fund] chunk #$chunkIdx",
            )
        }
        println("[fund] OK")
    } finally {
        try {
            sdk.disconnect()
        } catch (e: Exception) {
            System.err.println("[fund] disconnect warn: ${e.message}")
        }
    }
}

private suspend fun waitForBalanceIncrease(
    sdk: BreezSdk,
    currentBalance: ULong,
    timeoutMs: Long,
    pollLabel: String,
) {
    val startMs = System.currentTimeMillis()
    val deadline = startMs + timeoutMs
    var nextLogAtMs = startMs + 10_000
    while (System.currentTimeMillis() < deadline) {
        delay(5_000)
        val info = sdk.syncedInfo()
        if (info.balanceSats > currentBalance) return
        val now = System.currentTimeMillis()
        if (now >= nextLogAtMs) {
            val elapsedSec = (now - startMs) / 1000
            val timeoutSec = timeoutMs / 1000
            println("$pollLabel waiting for balance increase… elapsed=${elapsedSec}s/${timeoutSec}s (still $currentBalance sats)")
            nextLogAtMs = now + 10_000
        }
    }
    error("Balance did not increase within ${timeoutMs}ms (was $currentBalance sats)")
}

// --- seed-senders mode (top up sender pool from treasurer) ----------------

fun seedSenders(opts: Map<String, String>) = runBlocking {
    val mysqlUrl = opts["mysql-url"]
        ?: error("--mysql-url=mysql://user:pass@host:port/dbname is required")
    val masterSecret = opts["master-secret"]
        ?: System.getenv("MASTER_SECRET")
        ?: error("--master-secret=<hex-or-string> or MASTER_SECRET env var is required")
    val senderCount = opts["senders"]?.toIntOrNull() ?: 50
    val perSenderSats = opts["per-sender-sats"]?.toLongOrNull() ?: 5_000L
    val parallelism = opts["parallelism"]?.toIntOrNull() ?: 5

    require(senderCount > 0) { "--senders must be > 0" }
    require(perSenderSats > 0) { "--per-sender-sats must be > 0" }
    require(parallelism > 0) { "--parallelism must be > 0" }

    println(
        "[seed] senders=$senderCount  per-sender=$perSenderSats  parallel=$parallelism  " +
            "mysql=${maskPassword(mysqlUrl)}"
    )

    val config = benchConfig()
    val handlers = SharedHandlers.create(mysqlUrl)

    val treasurerSeed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, TREASURER_USER_ID))
    val treasurer = buildSdk(benchConfig(), treasurerSeed, handlers)

    try {
        // Lower bound on true balance (sync only adds incoming) — fine for the warning below.
        val treasurerInfo = treasurer.getInfo(GetInfoRequest(ensureSynced = false))
        val treasurerBalance = treasurerInfo.balanceSats.toLong()
        println("[seed] treasurer balance (cached): $treasurerBalance sats")
        val maxSpend = senderCount.toLong() * perSenderSats
        if (treasurerBalance < maxSpend) {
            System.err.println(
                "[seed] warning: treasurer has $treasurerBalance sats; up to $maxSpend may be needed " +
                    "if all senders are empty. Run 'make fund' first."
            )
        }

        val sem = Semaphore(parallelism)
        var fundedCount = 0
        var skippedCount = 0
        var failedCount = 0
        coroutineScope {
            for (i in 0 until senderCount) {
                launch {
                    sem.withPermit {
                        val outcome = try {
                            seedOneSender(
                                treasurer = treasurer,
                                senderIdx = i,
                                masterSecret = masterSecret,
                                config = config,
                                handlers = handlers,
                                perSenderSats = perSenderSats,
                            )
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            // Swallow per-sender failure; non-zero exit at the end, re-run is idempotent.
                            System.err.println("[seed] sender $i FAILED: ${e.message}")
                            SeedOutcome.FAILED
                        }
                        synchronized(this@runBlocking) {
                            when (outcome) {
                                SeedOutcome.FUNDED -> fundedCount++
                                SeedOutcome.SKIPPED -> skippedCount++
                                SeedOutcome.FAILED -> failedCount++
                            }
                        }
                    }
                }
            }
        }
        println("[seed] funded=$fundedCount  skipped=$skippedCount  failed=$failedCount")
        if (failedCount > 0) {
            error("[seed] $failedCount sender(s) still unfunded after this pass")
        }
        println("[seed] OK")
    } finally {
        try {
            treasurer.disconnect()
        } catch (e: Exception) {
            System.err.println("[seed] treasurer disconnect warn: ${e.message}")
        }
    }
}

private enum class SeedOutcome { FUNDED, SKIPPED, FAILED }

private suspend fun seedOneSender(
    treasurer: BreezSdk,
    senderIdx: Int,
    masterSecret: String,
    config: breez_sdk_spark.Config,
    handlers: SharedHandlers,
    perSenderSats: Long,
): SeedOutcome {
    val userId = senderUserId(senderIdx)
    val seed: Seed = Seed.Entropy(deriveSeedBytes(masterSecret, userId))
    val sender = buildSdk(config, seed, handlers)

    return try {
        val info = sender.syncedInfo()
        val balance = info.balanceSats.toLong()
        if (balance >= perSenderSats) {
            println("[seed] sender $senderIdx: $balance sats (≥ $perSenderSats, skip)")
            return SeedOutcome.SKIPPED
        }
        val sparkAddr = sender.receivePayment(
            ReceivePaymentRequest(paymentMethod = ReceivePaymentMethod.SparkAddress)
        ).paymentRequest
        val toSend = perSenderSats - balance
        println("[seed] sender $senderIdx: $balance sats → topping up by $toSend to $perSenderSats")

        val t0 = System.currentTimeMillis()
        val prepared = treasurer.prepareSendPayment(
            PrepareSendPaymentRequest(
                paymentRequest = sparkAddr,
                amount = BigInteger.fromLong(toSend),
            )
        )
        treasurer.sendPayment(SendPaymentRequest(prepareResponse = prepared))
        println("[seed] sender $senderIdx: treasurer send ${System.currentTimeMillis() - t0}ms")

        println("[seed] sender $senderIdx: waitForBalanceIncrease …")
        waitForBalanceIncrease(
            sender,
            balance.toULong(),
            timeoutMs = 60_000,
            pollLabel = "[seed] sender $senderIdx",
        )
        SeedOutcome.FUNDED
    } finally {
        try {
            sender.disconnect()
        } catch (e: Exception) {
            System.err.println("[seed] sender $senderIdx disconnect warn: ${e.message}")
        }
    }
}
