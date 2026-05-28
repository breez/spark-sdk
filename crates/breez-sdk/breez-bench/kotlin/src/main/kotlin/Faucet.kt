import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.time.Duration
import java.util.Base64

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

// Lightspark regtest faucet client. Reads FAUCET_URL / FAUCET_USERNAME / FAUCET_PASSWORD
// from the env. Per-call cap 50_000 / floor 1_000 sats — callers chunk larger top-ups.
object Faucet {
    private const val DEFAULT_URL = "https://api.lightspark.com/graphql/spark/rc"
    private const val MUTATION =
        "mutation RequestRegtestFunds(\$address: String!, \$amount_sats: Long!) { " +
            "request_regtest_funds(input: {address: \$address, amount_sats: \$amount_sats}) { " +
            "transaction_hash}}"

    const val MAX_PER_CALL_SATS: Long = 50_000
    const val MIN_PER_CALL_SATS: Long = 1_000

    private val httpClient: HttpClient by lazy {
        HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(15)).build()
    }
    private val jsonCodec = Json { ignoreUnknownKeys = true }

    /**
     * Sends `amountSats` to `address`, returning the on-chain transaction hash.
     * Retries with exponential backoff up to 3 times.
     */
    fun fundBitcoinAddress(address: String, amountSats: Long): String {
        require(amountSats in MIN_PER_CALL_SATS..MAX_PER_CALL_SATS) {
            "amountSats must be in [$MIN_PER_CALL_SATS, $MAX_PER_CALL_SATS]; got $amountSats"
        }
        val faucetUrl = System.getenv("FAUCET_URL") ?: DEFAULT_URL
        val username = System.getenv("FAUCET_USERNAME")
            ?: error("FAUCET_USERNAME env var is required")
        val password = System.getenv("FAUCET_PASSWORD")
            ?: error("FAUCET_PASSWORD env var is required")

        val body = buildJsonObject {
            put("operationName", "RequestRegtestFunds")
            put("variables", buildJsonObject {
                put("address", address)
                put("amount_sats", amountSats)
            })
            put("query", MUTATION)
        }.toString()

        val auth = Base64.getEncoder().encodeToString("$username:$password".toByteArray())
        val request = HttpRequest.newBuilder()
            .uri(URI.create(faucetUrl))
            .timeout(Duration.ofSeconds(30))
            .header("Content-Type", "application/json")
            .header("Authorization", "Basic $auth")
            .POST(HttpRequest.BodyPublishers.ofString(body))
            .build()

        var lastErr: Exception? = null
        val maxAttempts = 4
        for (attempt in 0 until maxAttempts) {
            if (attempt > 0) {
                Thread.sleep((1L shl attempt) * 1000L)
            }
            try {
                val resp = httpClient.send(request, HttpResponse.BodyHandlers.ofString())
                val payload = jsonCodec.parseToJsonElement(resp.body()).jsonObject
                val errors = payload["errors"]
                if (errors != null) {
                    throw RuntimeException("Faucet GraphQL error: $errors")
                }
                val txid = payload["data"]?.jsonObject
                    ?.get("request_regtest_funds")?.jsonObject
                    ?.get("transaction_hash")?.jsonPrimitive?.content
                    ?: throw RuntimeException("Unexpected faucet response: ${resp.body()}")
                return txid
            } catch (e: Exception) {
                lastErr = e
            }
        }
        throw lastErr ?: RuntimeException("Faucet call failed with no attempts recorded")
    }
}
