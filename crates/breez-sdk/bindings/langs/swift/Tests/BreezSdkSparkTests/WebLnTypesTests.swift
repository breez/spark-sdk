import XCTest
@testable import BreezSdkSpark

final class WebLnTypesTests: XCTestCase {

    func testLnurlTypeValues() {
        // Verify all LnurlType cases exist
        let pay: LnurlType = .pay
        let withdraw: LnurlType = .withdraw
        let auth: LnurlType = .auth

        XCTAssertNotEqual(pay, withdraw)
        XCTAssertNotEqual(pay, auth)
        XCTAssertNotEqual(withdraw, auth)
    }

    func testLnurlRequestPayType() {
        let request = LnurlRequest(
            type: .pay,
            domain: "example.com",
            minAmountSats: 1000,
            maxAmountSats: 100000,
            metadata: "[\"text/plain\", \"test\"]"
        )

        XCTAssertEqual(request.type, .pay)
        XCTAssertEqual(request.domain, "example.com")
        XCTAssertEqual(request.minAmountSats, 1000)
        XCTAssertEqual(request.maxAmountSats, 100000)
        XCTAssertEqual(request.metadata, "[\"text/plain\", \"test\"]")
        XCTAssertNil(request.defaultDescription)
    }

    func testLnurlRequestWithdrawType() {
        let request = LnurlRequest(
            type: .withdraw,
            domain: "service.com",
            minAmountSats: 100,
            maxAmountSats: 50000,
            defaultDescription: "Withdrawal"
        )

        XCTAssertEqual(request.type, .withdraw)
        XCTAssertEqual(request.domain, "service.com")
        XCTAssertEqual(request.minAmountSats, 100)
        XCTAssertEqual(request.maxAmountSats, 50000)
        XCTAssertEqual(request.defaultDescription, "Withdrawal")
        XCTAssertNil(request.metadata)
    }

    func testLnurlRequestAuthType() {
        let request = LnurlRequest(
            type: .auth,
            domain: "auth.example.com"
        )

        XCTAssertEqual(request.type, .auth)
        XCTAssertEqual(request.domain, "auth.example.com")
        XCTAssertNil(request.minAmountSats)
        XCTAssertNil(request.maxAmountSats)
        XCTAssertNil(request.metadata)
        XCTAssertNil(request.defaultDescription)
    }

    func testLnurlUserResponseApproved() {
        let response = LnurlUserResponse(
            approved: true,
            amountSats: 5000,
            comment: "Thanks!"
        )

        XCTAssertTrue(response.approved)
        XCTAssertEqual(response.amountSats, 5000)
        XCTAssertEqual(response.comment, "Thanks!")
    }

    func testLnurlUserResponseRejected() {
        let response = LnurlUserResponse(approved: false)

        XCTAssertFalse(response.approved)
        XCTAssertNil(response.amountSats)
        XCTAssertNil(response.comment)
    }
}
