package com.example.kotlinmpplib

import breez_sdk_spark.*

suspend fun parseLnurlAuth(client: BreezClient) {
    // ANCHOR: parse-lnurl-auth
    // LNURL-auth URL from a service
    // Can be in the form:
    // - lnurl1... (bech32 encoded)
    // - https://service.com/lnurl-auth?tag=login&k1=...
    val lnurlAuthUrl = "lnurl1..."

    when (val inputType = client.parse(lnurlAuthUrl)) {
        is InputType.LnurlAuth -> {
            val requestData = inputType.v1
            println("Domain: ${requestData.domain}")
            println("Action: ${requestData.action}")

            // Show domain to user and ask for confirmation
            // This is important for security
        }
        else -> {}
    }
    // ANCHOR_END: parse-lnurl-auth
}

suspend fun authenticate(client: BreezClient, requestData: LnurlAuthRequestDetails) {
    // ANCHOR: lnurl-auth
    // Perform LNURL authentication
    when (val result = client.lnurl().auth(requestData)) {
        is LnurlCallbackStatus.Ok -> {
            println("Authentication successful")
        }
        is LnurlCallbackStatus.ErrorStatus -> {
            println("Authentication failed: ${result.errorDetails.reason}")
        }
    }
    // ANCHOR_END: lnurl-auth
}
