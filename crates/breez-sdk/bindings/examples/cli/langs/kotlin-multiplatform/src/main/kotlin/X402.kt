import breez_sdk_spark.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.jline.reader.LineReader
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.time.Duration
import java.util.Base64

data class X402CliCommand(
    val name: String,
    val description: String,
    val run: suspend (sdk: BreezSdk, reader: LineReader, args: List<String>) -> Unit
)

val X402_COMMAND_NAMES = listOf(
    "fetch",
)

fun buildX402Registry(): Map<String, X402CliCommand> {
    return mapOf(
        "fetch" to X402CliCommand(
            "fetch",
            "Fetch a URL, paying its x402 Lightning challenge if the server asks for one",
            ::handleX402Fetch,
        ),
    )
}

suspend fun dispatchX402Command(
    args: List<String>,
    sdk: BreezSdk,
    registry: Map<String, X402CliCommand>,
    reader: LineReader,
) {
    if (args.isEmpty() || args[0] == "help") {
        println()
        println("x402 subcommands:")
        registry.keys.sorted().forEach { name ->
            val cmd = registry[name]!!
            println("  x402 %-30s %s".format(name, cmd.description))
        }
        println()
        return
    }

    val subName = args[0]
    val subArgs = args.drop(1)

    val cmd = registry[subName]
    if (cmd == null) {
        println("Unknown x402 subcommand: $subName. Use 'x402 help' for available commands.")
        return
    }

    try {
        cmd.run(sdk, reader, subArgs)
    } catch (e: Exception) {
        println("Error: ${e.message}")
    }
}

// ---------------------------------------------------------------------------
// x402 command handlers
// ---------------------------------------------------------------------------

private const val PAYMENT_REQUIRED_HEADER = "payment-required"
private const val PAYMENT_SIGNATURE_HEADER = "payment-signature"
private const val PAYMENT_RESPONSE_HEADER = "payment-response"

/** Times to pay the same challenge again while its payment is pending. Fetching
 * again instead would bring a new challenge with a new invoice to pay. */
private const val PENDING_RETRIES = 3

/** Times to send the paid request again while the server answers 402 without a
 * new challenge, meaning the payment may still settle. */
private const val SETTLE_RETRIES = 5
private const val SETTLE_RETRY_DELAY_MS = 2000L
private val REQUEST_TIMEOUT: Duration = Duration.ofSeconds(60)

suspend fun handleX402Fetch(sdk: BreezSdk, reader: LineReader, args: List<String>) {
    val fp = FlagParser(args)
    val url = fp.positional.getOrNull(0)
    val maxAmountSats = fp.getULong("max-amount-sats")
    if (url == null || maxAmountSats == null) {
        println("Usage: x402 fetch <url> --max-amount-sats <sats> [-X GET|POST] [-d <body>] [-H \"Name: value\"]...")
        return
    }
    val method = fp.getString("X", "method") ?: "GET"
    val body = fp.getString("d", "body")

    // HTTP/1.1 keeps the client from adding h2c upgrade headers of its own.
    val client = HttpClient.newBuilder()
        .version(HttpClient.Version.HTTP_1_1)
        .followRedirects(HttpClient.Redirect.NORMAL)
        .build()
    // The server hashes the headers it binds, so send every header
    // explicitly: anything the HTTP client added on its own would be
    // missing from the hash computed here.
    val headers = parseX402Headers(fp.getAll("H") + fp.getAll("header"))
    headers.putIfAbsent("accept", "*/*")

    val response = sendX402Request(client, method, url, headers, body)
    if (response.statusCode() != 402) {
        printX402Response(response)
        return
    }
    val paymentRequired = response.headers().firstValue(PAYMENT_REQUIRED_HEADER).orElse(null)
        ?: throw Exception("402 response has no PAYMENT-REQUIRED header")

    val x402 = sdk.getX402()
    val request = PayX402Request(
        paymentRequired = paymentRequired,
        request = X402Request.Http(
            method = method,
            url = url,
            headers = headers.map { (name, value) -> X402Header(name = name, value = value) },
            body = body?.toByteArray(Charsets.UTF_8),
        ),
        maxAmount = X402PaymentAmount.Lightning(amountSats = maxAmountSats),
        conversionOptions = null,
        completionTimeoutSecs = null,
    )
    var paid = x402.pay(request)
    for (i in 0 until PENDING_RETRIES) {
        if (paid.status == X402PaymentStatus.PAID) break
        println("Payment is pending, waiting for it to complete...")
        paid = x402.pay(request)
    }
    printValue(paid)
    val signature = paid.paymentSignature
    if (signature == null) {
        println("Payment is still pending. Check its status with list-payments.")
        return
    }

    headers[PAYMENT_SIGNATURE_HEADER] = signature
    var paidResponse = sendX402Request(client, method, url, headers, body)
    for (i in 0 until SETTLE_RETRIES) {
        if (paidResponse.statusCode() != 402 ||
            paidResponse.headers().firstValue(PAYMENT_REQUIRED_HEADER).isPresent
        ) {
            break
        }
        println("Payment is not settled yet, sending it again...")
        delay(SETTLE_RETRY_DELAY_MS)
        paidResponse = sendX402Request(client, method, url, headers, body)
    }
    printX402Response(paidResponse)
}

private fun parseX402Headers(lines: List<String>): MutableMap<String, String> {
    val headers = linkedMapOf<String, String>()
    for (line in lines) {
        val colon = line.indexOf(':')
        if (colon < 0) {
            throw Exception("header \"$line\" is not \"Name: value\"")
        }
        headers[line.substring(0, colon).trim().lowercase()] = line.substring(colon + 1).trim()
    }
    return headers
}

private suspend fun sendX402Request(
    client: HttpClient,
    method: String,
    url: String,
    headers: Map<String, String>,
    body: String?,
): HttpResponse<String> {
    val builder = HttpRequest.newBuilder(URI.create(url)).timeout(REQUEST_TIMEOUT)
    headers.forEach { (name, value) -> builder.header(name, value) }
    val request = when (method) {
        "GET" -> builder.GET()
        "POST" -> builder.POST(
            body?.let { HttpRequest.BodyPublishers.ofString(it) } ?: HttpRequest.BodyPublishers.noBody()
        )
        else -> throw Exception("unsupported method $method: use GET or POST")
    }.build()
    val response = withContext(Dispatchers.IO) {
        client.send(request, HttpResponse.BodyHandlers.ofString())
    }
    val status = response.statusCode()
    if (status in listOf(301, 302, 303, 307, 308)) {
        val location = response.headers().firstValue("location").orElse("<no location>")
        throw Exception("endpoint returned a $status redirect to $location, which this client does not follow")
    }
    return response
}

private fun printX402Response(response: HttpResponse<String>) {
    println("HTTP ${response.statusCode()}")
    response.headers().firstValue(PAYMENT_RESPONSE_HEADER).orElse(null)
        ?.let { runCatching { Base64.getDecoder().decode(it) }.getOrNull() }
        ?.let { println("PAYMENT-RESPONSE: ${String(it, Charsets.UTF_8)}") }
    println(response.body())
}
