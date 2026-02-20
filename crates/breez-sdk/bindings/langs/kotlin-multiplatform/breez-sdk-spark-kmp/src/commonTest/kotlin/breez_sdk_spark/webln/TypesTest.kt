package breez_sdk_spark.webln

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlin.test.assertFalse

class TypesTest {

    @Test
    fun testLnurlTypeValues() {
        // Verify all LnurlType enum values exist
        val pay = LnurlType.PAY
        val withdraw = LnurlType.WITHDRAW
        val auth = LnurlType.AUTH

        assertNotEquals(pay, withdraw)
        assertNotEquals(pay, auth)
        assertNotEquals(withdraw, auth)
    }

    @Test
    fun testLnurlRequestPayType() {
        val request = LnurlRequest(
            type = LnurlType.PAY,
            domain = "example.com",
            minAmountSats = 1000,
            maxAmountSats = 100000,
            metadata = """[["text/plain", "test"]]"""
        )

        assertEquals(LnurlType.PAY, request.type)
        assertEquals("example.com", request.domain)
        assertEquals(1000, request.minAmountSats)
        assertEquals(100000, request.maxAmountSats)
        assertEquals("""[["text/plain", "test"]]""", request.metadata)
        assertNull(request.defaultDescription)
    }

    @Test
    fun testLnurlRequestWithdrawType() {
        val request = LnurlRequest(
            type = LnurlType.WITHDRAW,
            domain = "service.com",
            minAmountSats = 100,
            maxAmountSats = 50000,
            defaultDescription = "Withdrawal"
        )

        assertEquals(LnurlType.WITHDRAW, request.type)
        assertEquals("service.com", request.domain)
        assertEquals(100, request.minAmountSats)
        assertEquals(50000, request.maxAmountSats)
        assertEquals("Withdrawal", request.defaultDescription)
        assertNull(request.metadata)
    }

    @Test
    fun testLnurlRequestAuthType() {
        val request = LnurlRequest(
            type = LnurlType.AUTH,
            domain = "auth.example.com"
        )

        assertEquals(LnurlType.AUTH, request.type)
        assertEquals("auth.example.com", request.domain)
        assertNull(request.minAmountSats)
        assertNull(request.maxAmountSats)
        assertNull(request.metadata)
        assertNull(request.defaultDescription)
    }

    @Test
    fun testLnurlUserResponseApproved() {
        val response = LnurlUserResponse(
            approved = true,
            amountSats = 5000,
            comment = "Thanks!"
        )

        assertTrue(response.approved)
        assertEquals(5000, response.amountSats)
        assertEquals("Thanks!", response.comment)
    }

    @Test
    fun testLnurlUserResponseRejected() {
        val response = LnurlUserResponse(approved = false)

        assertFalse(response.approved)
        assertNull(response.amountSats)
        assertNull(response.comment)
    }

    @Test
    fun testWebLnErrorCodes() {
        assertEquals("USER_REJECTED", WebLnErrorCode.USER_REJECTED)
        assertEquals("PROVIDER_NOT_ENABLED", WebLnErrorCode.PROVIDER_NOT_ENABLED)
        assertEquals("INSUFFICIENT_FUNDS", WebLnErrorCode.INSUFFICIENT_FUNDS)
        assertEquals("INVALID_PARAMS", WebLnErrorCode.INVALID_PARAMS)
        assertEquals("UNSUPPORTED_METHOD", WebLnErrorCode.UNSUPPORTED_METHOD)
        assertEquals("INTERNAL_ERROR", WebLnErrorCode.INTERNAL_ERROR)
    }
}
