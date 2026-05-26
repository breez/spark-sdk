import Foundation

/// Relying Party name used by the zero-config ``PasskeyClient``
/// convenience initializer. Surfaces in some credential-manager UIs
/// (iCloud Keychain, Google Password Manager). Apps that want their own
/// RP name build a ``PasskeyProvider`` explicitly and inject it through
/// ``PasskeyClientBuilder``.
private let defaultRpName = "Breez"

/// Builder for a ``PasskeyClient`` backed by a caller-supplied
/// ``PrfProvider``.
///
/// Use this when you need a configured ``PasskeyProvider`` (custom
/// `rpId` / `rpName`, a `credentialRegistry`, rotating `userName`,
/// timeout overrides) or a fully custom PRF backend. For the zero-config
/// Breez-RP case, use ``PasskeyClient/init(breezApiKey:config:)``.
///
/// ```swift
/// let provider = PasskeyProvider(rpId: rpId, rpName: rpName, credentialRegistry: registry)
/// let client = PasskeyClientBuilder(breezApiKey: apiKey)
///     .withPrfProvider(provider)
///     .build()
/// ```
public class PasskeyClientBuilder {
    private let breezApiKey: String?
    private let config: PasskeyConfig?
    private var provider: (any PrfProvider)?

    /// - Parameters:
    ///   - breezApiKey: Breez relay key for authenticated (NIP-42) label
    ///     storage. Pass `nil` for public relays only.
    ///   - config: Optional ``PasskeyConfig`` (e.g. a default label).
    public init(breezApiKey: String? = nil, config: PasskeyConfig? = nil) {
        self.breezApiKey = breezApiKey
        self.config = config
    }

    /// Inject the ``PrfProvider`` the client derives seeds through. The
    /// built-in ``PasskeyProvider`` or any custom implementation is
    /// accepted.
    @discardableResult
    public func withPrfProvider(_ provider: any PrfProvider) -> PasskeyClientBuilder {
        self.provider = provider
        return self
    }

    /// Construct the client. Falls back to a default ``PasskeyProvider``
    /// on the Breez RP when no provider was injected.
    public func build() -> PasskeyClient {
        let resolved = provider
            ?? PasskeyProvider(rpId: PasskeyProvider.BREEZ_RP_ID, rpName: defaultRpName)
        return PasskeyClient(prfProvider: resolved, breezApiKey: breezApiKey, config: config)
    }
}

public extension PasskeyClient {
    /// Zero-config client wired to the built-in ``PasskeyProvider`` on
    /// the Breez shared RP (`keys.breez.technology`), so a
    /// Breez-registered app needs only its relay key.
    ///
    /// Apps with their own RP, a credential registry, or a custom PRF
    /// backend build the provider themselves and inject it through
    /// ``PasskeyClientBuilder``.
    ///
    /// - Parameters:
    ///   - breezApiKey: Breez relay key for authenticated (NIP-42) label
    ///     storage. Pass `nil` for public relays only.
    ///   - config: Optional ``PasskeyConfig`` (e.g. a default label).
    convenience init(breezApiKey: String?, config: PasskeyConfig? = nil) {
        let provider = PasskeyProvider(rpId: PasskeyProvider.BREEZ_RP_ID, rpName: defaultRpName)
        self.init(prfProvider: provider, breezApiKey: breezApiKey, config: config)
    }
}
