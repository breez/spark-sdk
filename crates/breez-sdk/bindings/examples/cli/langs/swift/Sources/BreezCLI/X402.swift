import Foundation
import BreezSdkSpark

// MARK: - x402 command names (for REPL completion)

let x402CommandNames: [String] = [
    "x402 fetch",
]

private let paymentRequiredHeader = "payment-required"
private let paymentSignatureHeader = "payment-signature"
private let paymentResponseHeader = "payment-response"
/// Times to pay the same challenge again while its payment is pending. Fetching
/// again instead would bring a new challenge with a new invoice to pay.
private let pendingRetries = 3
/// Times to send the paid request again while the server answers 402 without a
/// new challenge, meaning the payment may still settle.
private let settleRetries = 5
private let settleRetryDelayNanos: UInt64 = 2_000_000_000

// MARK: - Dispatch

func dispatchX402Command(_ args: [String], sdk: BreezSdk) async {
    if args.isEmpty || args[0] == "help" {
        printX402Help()
        return
    }

    let subName = args[0]
    let subArgs = Array(args.dropFirst())

    do {
        switch subName {
        case "fetch":
            try await handleX402Fetch(sdk, subArgs)
        default:
            print("Unknown x402 subcommand: \(subName). Use 'x402 help' for available commands.")
        }
    } catch {
        print("Error: \(error)")
    }
}

// MARK: - Help

private func printX402Help() {
    print("\nx402 subcommands:")
    print("  x402 \("fetch".padding(toLength: 30, withPad: " ", startingAt: 0))Fetch a URL, paying its x402 Lightning challenge if the server asks for one")
    print()
}

// MARK: - Handlers

// --- fetch ---

private func handleX402Fetch(_ sdk: BreezSdk, _ args: [String]) async throws {
    let fp = FlagParser(args)
    guard let url = fp.positional.first,
          let maxAmountSats = fp.get("max-amount-sats").flatMap({ UInt64($0) }) else {
        print("Usage: x402 fetch <url> --max-amount-sats <n> [-X|--method GET|POST] [-d|--body <body>] [-H|--header \"Name: value\"]...")
        return
    }
    let method = fp.get("X", "method") ?? "GET"
    let body = fp.get("d", "body")

    // The server hashes the headers it binds, so send every header explicitly:
    // anything the HTTP client added on its own would be missing from the hash
    // computed here.
    var headers = try parseHeaders(fp.getAll("H", "header"))
    if headers["accept"] == nil {
        headers["accept"] = "*/*"
    }
    // URLSession always adds this to a POST body that has none, so name it here
    // to keep it in the headers the payment is for.
    if method == "POST", body != nil, headers["content-type"] == nil {
        headers["content-type"] = "application/x-www-form-urlencoded"
    }

    let response = try await send(method: method, url: url, headers: headers, body: body)
    if response.status != 402 {
        printResponse(response)
        return
    }
    guard let paymentRequired = response.header(paymentRequiredHeader) else {
        throw X402CliError(description: "402 response has no PAYMENT-REQUIRED header")
    }

    let x402 = sdk.getX402()
    let request = PayX402Request(
        paymentRequired: paymentRequired,
        request: X402Request.http(
            method: method,
            url: url,
            headers: headers.sorted { $0.key < $1.key }.map { X402Header(name: $0.key, value: $0.value) },
            body: body.map { Data($0.utf8) }
        ),
        maxAmount: X402PaymentAmount.lightning(amountSats: maxAmountSats),
        conversionOptions: nil,
        completionTimeoutSecs: nil
    )
    var paid = try await x402.pay(request: request)
    for _ in 0..<pendingRetries {
        if case .paid = paid.status { break }
        print("Payment is pending, waiting for it to complete...")
        paid = try await x402.pay(request: request)
    }
    printValue(paid)
    guard let signature = paid.paymentSignature else {
        print("Payment is still pending. Check its status with list-payments.")
        return
    }

    headers[paymentSignatureHeader] = signature
    var paidResponse = try await send(method: method, url: url, headers: headers, body: body)
    for _ in 0..<settleRetries {
        if paidResponse.status != 402 || paidResponse.header(paymentRequiredHeader) != nil { break }
        print("Payment is not settled yet, sending it again...")
        try await Task.sleep(nanoseconds: settleRetryDelayNanos)
        paidResponse = try await send(method: method, url: url, headers: headers, body: body)
    }
    printResponse(paidResponse)
}

// MARK: - HTTP helpers

private struct X402CliError: Error, CustomStringConvertible {
    let description: String
}

private struct FetchResponse {
    let status: Int
    let http: HTTPURLResponse
    let body: String

    func header(_ name: String) -> String? {
        http.value(forHTTPHeaderField: name)
    }
}

private let fetchSession: URLSession = {
    let config = URLSessionConfiguration.ephemeral
    config.urlCache = nil
    config.requestCachePolicy = .reloadIgnoringLocalCacheData
    return URLSession(configuration: config)
}()

private func parseHeaders(_ lines: [String]) throws -> [String: String] {
    var headers: [String: String] = [:]
    for line in lines {
        guard let colon = line.firstIndex(of: ":") else {
            throw X402CliError(description: "header \"\(line)\" is not \"Name: value\"")
        }
        let name = line[..<colon].trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let value = line[line.index(after: colon)...].trimmingCharacters(in: .whitespacesAndNewlines)
        headers[name] = value
    }
    return headers
}

private func send(method: String, url: String, headers: [String: String], body: String?) async throws -> FetchResponse {
    guard method == "GET" || method == "POST" else {
        throw X402CliError(description: "unsupported method \(method): use GET or POST")
    }
    guard let target = URL(string: url) else {
        throw X402CliError(description: "invalid URL \(url)")
    }
    var req = URLRequest(url: target)
    req.httpMethod = method
    for (name, value) in headers {
        req.setValue(value, forHTTPHeaderField: name)
    }
    if method == "POST", let body {
        req.httpBody = Data(body.utf8)
    }
    let (data, response) = try await fetchSession.data(for: req)
    guard let http = response as? HTTPURLResponse else {
        throw X402CliError(description: "not an HTTP response")
    }
    // A redirect URLSession could not follow, such as one without a Location.
    if [301, 302, 303, 307, 308].contains(http.statusCode) {
        let location = http.value(forHTTPHeaderField: "location") ?? "<no location>"
        throw X402CliError(description: "endpoint returned a \(http.statusCode) redirect to \(location), which this client does not follow")
    }
    return FetchResponse(status: http.statusCode, http: http, body: String(decoding: data, as: UTF8.self))
}

private func printResponse(_ response: FetchResponse) {
    print("HTTP \(response.status)")
    if let settlement = response.header(paymentResponseHeader),
       let json = Data(base64Encoded: settlement) {
        print("PAYMENT-RESPONSE: \(String(decoding: json, as: UTF8.self))")
    }
    print(response.body)
}
