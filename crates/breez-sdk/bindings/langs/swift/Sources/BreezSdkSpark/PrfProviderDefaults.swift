import Foundation

/// Swift-side default implementations for `PrfProvider` methods that
/// have a Rust-side default but are surfaced as required-looking
/// methods in the UniFFI-generated protocol.
///
/// Custom `PrfProvider` implementations (CLI tools, file-backed
/// providers, hardware HSMs) inherit these defaults so adding new
/// methods to the trait does not require every host to update.
/// Built-in providers (the SDK's `PasskeyProvider`) override the
/// defaults with platform-specific fast paths where available.
public extension PrfProvider {
    /// Default loop fallback for bulk PRF derivation. Produces N
    /// authenticator ceremonies for N salts. Built-in
    /// [`PasskeyProvider`] overrides this with the WebAuthn PRF
    /// dual-salt fast path on iOS 18+, collapsing two derivations
    /// into a single ceremony.
    ///
    /// Output ordering matches input ordering.
    func derivePrfSeeds(salts: [String]) async throws -> [Data] {
        var out: [Data] = []
        out.reserveCapacity(salts.count)
        for salt in salts {
            let seed = try await derivePrfSeed(salt: salt)
            out.append(seed)
        }
        return out
    }
}
