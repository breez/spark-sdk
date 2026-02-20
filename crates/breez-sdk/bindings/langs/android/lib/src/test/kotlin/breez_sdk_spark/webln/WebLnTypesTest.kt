package breez_sdk_spark.webln

import org.junit.Assert.*
import org.junit.Test

class WebLnTypesTest {

    @Test
    fun `LnurlType has all expected values`() {
        val values = LnurlType.values()
        assertEquals(3, values.size)
        assertTrue(values.contains(LnurlType.PAY))
        assertTrue(values.contains(LnurlType.WITHDRAW))
        assertTrue(values.contains(LnurlType.AUTH))
    }

    @Test
    fun `LnurlRequest can be created for pay type`() {
        val request = LnurlRequest(
            type = LnurlType.PAY,
            domain = "example.com",
            minAmountSats = 1000,
            maxAmountSats = 100000,
            metadata = """[["text/plain", "test"]]"""
        )

        assertEquals(LnurlType.PAY, request.type)
        assertEquals("example.com", request.domain)
        assertEquals(1000L, request.minAmountSats)
        assertEquals(100000L, request.maxAmountSats)
        assertEquals("""[["text/plain", "test"]]""", request.metadata)
        assertNull(request.defaultDescription)
    }

    @Test
    fun `LnurlRequest can be created for withdraw type`() {
        val request = LnurlRequest(
            type = LnurlType.WITHDRAW,
            domain = "service.com",
            minAmountSats = 100,
            maxAmountSats = 50000,
            defaultDescription = "Withdrawal"
        )

        assertEquals(LnurlType.WITHDRAW, request.type)
        assertEquals("service.com", request.domain)
        assertEquals(100L, request.minAmountSats)
        assertEquals(50000L, request.maxAmountSats)
        assertEquals("Withdrawal", request.defaultDescription)
        assertNull(request.metadata)
    }

    @Test
    fun `LnurlRequest can be created for auth type with minimal fields`() {
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
    fun `LnurlUserResponse can be created with approval and amount`() {
        val response = LnurlUserResponse(
            approved = true,
            amountSats = 5000,
            comment = "Thanks!"
        )

        assertTrue(response.approved)
        assertEquals(5000L, response.amountSats)
        assertEquals("Thanks!", response.comment)
    }

    @Test
    fun `LnurlUserResponse can be created with rejection`() {
        val response = LnurlUserResponse(approved = false)

        assertFalse(response.approved)
        assertNull(response.amountSats)
        assertNull(response.comment)
    }
}
