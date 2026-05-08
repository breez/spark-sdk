import Foundation
import Security

/// iCloud-synced keychain store for the credential IDs of every passkey
/// this device has ever registered against a given RP.
///
/// Used to populate `excludeCredentials` on subsequent passkey-registration
/// requests so the platform refuses to create a duplicate, even after the
/// app has been uninstalled and reinstalled (which wipes localStorage but
/// not the iCloud-synced keychain item).
///
/// Storage shape: one keychain item per RP ID. The item is a generic
/// password whose attribute service is `"breez.spark.passkey.knownCredentials"`
/// and whose attribute account is the RP ID. The data blob is a
/// JSON-encoded array of base64-encoded credential IDs.
///
/// `kSecAttrSynchronizable=true` opts the item into iCloud Keychain sync
/// so it survives device reinstall and replicates across the user's
/// other devices signed into the same Apple ID.
@available(iOS 18.0, macOS 15.0, *)
public struct KnownCredentialsStore {
    private static let service = "breez.spark.passkey.knownCredentials"

    /// Read the persisted list of base64-encoded credential IDs for `rpId`.
    /// Returns an empty array if the keychain item is missing or invalid.
    public static func read(rpId: String) -> [String] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: rpId,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]

        var item: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &item)

        guard status == errSecSuccess, let data = item as? Data else {
            return []
        }

        guard let decoded = try? JSONDecoder().decode([String].self, from: data) else {
            // Corrupt or stale shape: surface as empty rather than crash.
            return []
        }
        return decoded
    }

    /// Append `credentialId` (base64-encoded) to the persisted list for
    /// `rpId`. No-op if already present. Creates the keychain item if it
    /// doesn't exist.
    public static func add(credentialId: String, rpId: String) {
        var existing = read(rpId: rpId)
        if existing.contains(credentialId) { return }
        existing.append(credentialId)
        write(ids: existing, rpId: rpId)
    }

    /// Drop a single `credentialId` from the persisted list for `rpId`.
    /// No-op if absent. Used by the switch-failure recovery path so a
    /// deleted passkey stops appearing in the management list while the
    /// rest of the user's known credentials remain tracked.
    public static func remove(credentialId: String, rpId: String) {
        let existing = read(rpId: rpId)
        let filtered = existing.filter { $0 != credentialId }
        if filtered.count == existing.count { return }
        if filtered.isEmpty {
            // Avoid persisting an empty list — clear the item entirely so
            // a fresh add() takes the insert path rather than the
            // update-an-empty-blob path.
            clear(rpId: rpId)
        } else {
            write(ids: filtered, rpId: rpId)
        }
    }

    /// Clear the persisted list for `rpId`. Used by the deletion-recovery
    /// flow when the platform reports `CredentialNotFound` on a sign-in
    /// attempt: the user has manually deleted the passkey from
    /// Settings → Passwords, so our stale list of IDs is no longer
    /// meaningful.
    public static func clear(rpId: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: rpId,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
        ]
        SecItemDelete(query as CFDictionary)
    }

    // MARK: - Private

    private static func write(ids: [String], rpId: String) {
        guard let data = try? JSONEncoder().encode(ids) else {
            return
        }

        let baseQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: rpId,
            kSecAttrSynchronizable as String: kSecAttrSynchronizableAny,
        ]

        // Try to update an existing item first; if missing, add a new one.
        let updateAttrs: [String: Any] = [
            kSecValueData as String: data,
        ]
        var status = SecItemUpdate(baseQuery as CFDictionary, updateAttrs as CFDictionary)

        if status == errSecItemNotFound {
            // Insert. Add the synchronizable + accessibility attributes here
            // (they're not allowed in update-attrs and we need them at
            // insert time to opt the item into iCloud Keychain sync).
            var addQuery = baseQuery
            addQuery[kSecValueData as String] = data
            addQuery[kSecAttrSynchronizable as String] = kCFBooleanTrue
            addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
            status = SecItemAdd(addQuery as CFDictionary, nil)
        }

        if status != errSecSuccess {
            // Best-effort: swallow the failure. The next createPasskey will
            // attempt to write again, and excludeCredentials will simply be
            // shorter than ideal until then. We don't fail the parent call
            // because the registration itself already succeeded.
            NSLog("[KnownCredentialsStore] write failed for rpId=%@, status=%d", rpId, status)
        }
    }
}
