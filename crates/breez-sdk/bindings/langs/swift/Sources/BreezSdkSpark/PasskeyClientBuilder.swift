import Foundation

/// Builder for a ``PasskeyClient`` backed by a caller-supplied
/// ``PrfProvider``.
///
/// Use this when you need a fully custom PRF backend or a
/// ``PasskeyProvider`` configured beyond `rpId` / `rpName` (a
/// `credentialRegistry`, rotating `userName`, timeout overrides). For
/// the zero-config or RP-only case, use
/// ``PasskeyClient/init(breezApiKey:rpId:rpName:config:)``.
///
/// ```swift
/// let provider = PasskeyProvider(rpId: rpId, rpName: rpName, credentialRegistry: registry)
/// let client = PasskeyClientBuilder(breezApiKey: apiKey)
///     .withPrfProvider(provider)
///     .build()
/// ```
public class PasskeyClientBuilder {
    private let breezApiKey: String?
    private let rpId: String
    private let rpName: String
    private let config: PasskeyConfig?
    private var provider: (any PrfProvider)?

    /// - Parameters:
    ///   - breezApiKey: Breez relay key for authenticated (NIP-42) label
    ///     storage. Pass `nil` for public relays only.
    ///   - rpId: Relying Party ID for the default provider. Defaults to
    ///     ``PasskeyProvider/BREEZ_RP_ID``. Ignored when a provider is
    ///     injected via ``withPrfProvider(_:)``.
    ///   - rpName: Relying Party name for the default provider. Defaults
    ///     to ``PasskeyProvider/defaultRpName``. Ignored when a provider
    ///     is injected.
    ///   - config: Optional ``PasskeyConfig`` (e.g. a default label).
    public init(
        breezApiKey: String? = nil,
        rpId: String = PasskeyProvider.BREEZ_RP_ID,
        rpName: String = PasskeyProvider.defaultRpName,
        config: PasskeyConfig? = nil
    ) {
        self.breezApiKey = breezApiKey
        self.rpId = rpId
        self.rpName = rpName
        self.config = config
    }

    /// Inject the ``PrfProvider`` the client derives seeds through. The
    /// built-in ``PasskeyProvider`` or any custom implementation is
    /// accepted. Supersedes the `rpId` / `rpName` arguments (the injected
    /// provider owns its RP).
    @discardableResult
    public func withPrfProvider(_ provider: any PrfProvider) -> PasskeyClientBuilder {
        self.provider = provider
        return self
    }

    /// Construct the client. Falls back to a default ``PasskeyProvider``
    /// on the configured `rpId` / `rpName` when no provider was injected.
    public func build() -> PasskeyClient {
        let resolved = provider ?? PasskeyProvider(rpId: rpId, rpName: rpName)
        return PasskeyClient(prfProvider: resolved, breezApiKey: breezApiKey, config: config)
    }
}

public extension PasskeyClient {
    /// Client wired to the built-in ``PasskeyProvider``. Defaults to the
    /// Breez shared RP (`keys.breez.technology`), so a Breez-registered
    /// app needs only its relay key; pass `rpId` / `rpName` to use your
    /// own RP.
    ///
    /// Apps that need a credential registry or a custom PRF backend build
    /// the provider themselves and inject it through
    /// ``PasskeyClientBuilder``.
    ///
    /// - Parameters:
    ///   - breezApiKey: Breez relay key for authenticated (NIP-42) label
    ///     storage. Pass `nil` for public relays only.
    ///   - rpId: Relying Party ID. Defaults to ``PasskeyProvider/BREEZ_RP_ID``.
    ///   - rpName: Relying Party name. Defaults to ``PasskeyProvider/defaultRpName``.
    ///   - config: Optional ``PasskeyConfig`` (e.g. a default label).
    convenience init(
        breezApiKey: String?,
        rpId: String = PasskeyProvider.BREEZ_RP_ID,
        rpName: String = PasskeyProvider.defaultRpName,
        config: PasskeyConfig? = nil
    ) {
        self.init(
            prfProvider: PasskeyProvider(rpId: rpId, rpName: rpName),
            breezApiKey: breezApiKey,
            config: config
        )
    }
}
