import Foundation

/// Builder for a ``PasskeyClient`` backed by a caller-supplied
/// ``PrfProvider``.
///
/// Use this when you need a fully custom PRF backend or a
/// ``PasskeyProvider`` configured beyond `rpId` / `rpName` (a
/// `credentialRegistry`, rotating `userName`, timeout overrides). For the
/// zero-config or RP-only case, use
/// ``PasskeyClient/init(breezApiKey:config:)`` and set `rpId` / `rpName`
/// on the ``PasskeyConfig``.
@available(iOS 18.0, macOS 15.0, *)
public class PasskeyClientBuilder {
    private let breezApiKey: String?
    private let config: PasskeyConfig?
    private var provider: (any PrfProvider)?

    /// - Parameters:
    ///   - breezApiKey: Breez relay key for authenticated (NIP-42) label
    ///     storage. Pass `nil` for public relays only.
    ///   - config: Passkey client config. `rpId` / `rpName` configure the
    ///     default provider (ignored when one is injected via
    ///     ``withPrfProvider(_:)``); `defaultLabel` is the label-store
    ///     default.
    public init(breezApiKey: String? = nil, config: PasskeyConfig? = nil) {
        self.breezApiKey = breezApiKey
        self.config = config
    }

    /// Inject the ``PrfProvider`` the client derives seeds through. The
    /// built-in ``PasskeyProvider`` or any custom implementation is
    /// accepted. Supersedes the config's `rpId` / `rpName` (the injected
    /// provider owns its RP).
    @discardableResult
    public func withPrfProvider(_ provider: any PrfProvider) -> PasskeyClientBuilder {
        self.provider = provider
        return self
    }

    /// Construct the client. Falls back to a default ``PasskeyProvider``
    /// on the config's `rpId` / `rpName` (default: the Breez RP) when no
    /// provider was injected.
    public func build() -> PasskeyClient {
        let resolved = provider ?? PasskeyProvider(
            rpId: config?.rpId ?? PasskeyProvider.BREEZ_RP_ID,
            rpName: config?.rpName ?? PasskeyProvider.defaultRpName
        )
        return PasskeyClient(prfProvider: resolved, breezApiKey: breezApiKey, config: config)
    }
}

public extension PasskeyClient {
    /// Client wired to the built-in ``PasskeyProvider``. Defaults to the
    /// Breez shared RP (`keys.breez.technology`), so a Breez-registered
    /// app needs only its relay key; set `rpId` / `rpName` on the
    /// ``PasskeyConfig`` to use your own RP.
    ///
    /// Apps that need a credential registry or a custom PRF backend build
    /// the provider themselves and inject it through
    /// ``PasskeyClientBuilder``.
    ///
    /// - Parameters:
    ///   - breezApiKey: Breez relay key for authenticated (NIP-42) label
    ///     storage. Pass `nil` for public relays only.
    ///   - config: Passkey client config (`rpId` / `rpName` / `defaultLabel`).
    @available(iOS 18.0, macOS 15.0, *)
    convenience init(breezApiKey: String?, config: PasskeyConfig? = nil) {
        self.init(
            prfProvider: PasskeyProvider(
                rpId: config?.rpId ?? PasskeyProvider.BREEZ_RP_ID,
                rpName: config?.rpName ?? PasskeyProvider.defaultRpName
            ),
            breezApiKey: breezApiKey,
            config: config
        )
    }
}
