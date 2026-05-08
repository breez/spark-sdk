package technology.breez.spark.passkey

import android.content.Context
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import com.google.android.gms.auth.blockstore.Blockstore
import com.google.android.gms.auth.blockstore.BlockstoreClient
import com.google.android.gms.auth.blockstore.DeleteBytesRequest
import com.google.android.gms.auth.blockstore.RetrieveBytesRequest
import com.google.android.gms.auth.blockstore.StoreBytesData
import kotlinx.coroutines.tasks.await
import org.json.JSONArray

/**
 * Cross-device-synced store of the credential IDs registered for an RP.
 * Backs `excludeCredentialIds` on registration and `allowCredentialIds`
 * on assertion. Block Store is primary (Google-account synced),
 * EncryptedSharedPreferences is the local fallback. Writes go to both,
 * reads union them.
 */
public object KnownCredentialsStore {
    private const val TAG = "KnownCredentialsStore"
    private const val BLOCKSTORE_KEY_PREFIX = "breez.spark.passkey.knownCredentials."
    private const val ESP_FILE = "breez.spark.passkey.knownCredentials"
    private const val ESP_KEY_PREFIX = "credentials."

    /**
     * Read the persisted set of base64-encoded credential IDs for [rpId].
     * Returns an empty list when neither store has an entry, or both
     * fail to decode.
     */
    public suspend fun read(context: Context, rpId: String): List<String> {
        val seen = LinkedHashSet<String>()
        readFromBlockStore(context, rpId)?.forEach { seen.add(it) }
        readFromEsp(context, rpId).forEach { seen.add(it) }
        return seen.toList()
    }

    /**
     * Add [credentialIdBase64] to the persisted set for [rpId]. No-op if
     * already present in either store. Writes to both Block Store and
     * EncryptedSharedPreferences so a single-store outage does not
     * silently drop the entry.
     */
    public suspend fun add(context: Context, credentialIdBase64: String, rpId: String) {
        val current = LinkedHashSet(read(context, rpId))
        if (!current.add(credentialIdBase64)) return
        val encoded = encodeList(current.toList())
        writeToBlockStore(context, rpId, encoded)
        writeToEsp(context, rpId, encoded)
    }

    /** Local ESP-only variant of [add]. Pair with [syncBlockStore]. */
    public fun addLocal(context: Context, credentialIdBase64: String, rpId: String) {
        val current = readFromEsp(context, rpId).toMutableSet()
        if (!current.add(credentialIdBase64)) return
        writeToEsp(context, rpId, encodeList(current.toList()))
    }

    /** Mirror local entries up to Block Store. Idempotent. */
    public suspend fun syncBlockStore(context: Context, rpId: String) {
        val local = readFromEsp(context, rpId)
        if (local.isEmpty()) return
        val cloud = readFromBlockStore(context, rpId) ?: emptyList()
        val merged = LinkedHashSet<String>().apply {
            addAll(cloud)
            addAll(local)
        }
        if (merged.size == cloud.size) return
        writeToBlockStore(context, rpId, encodeList(merged.toList()))
    }

    /**
     * Clear the persisted set for [rpId] from both stores. Used by the
     * deletion-recovery flow when sign-in returns `CREDENTIAL_NOT_FOUND`:
     * the platform no longer has the passkey, so the cached IDs are
     * stale.
     */
    public suspend fun clear(context: Context, rpId: String) {
        clearBlockStore(context, rpId)
        clearEsp(context, rpId)
    }

    /**
     * Drop a single [credentialIdBase64] from the persisted set for
     * [rpId]. No-op if absent. Used by the switch-failure recovery
     * path so a deleted passkey stops appearing in the management list
     * while the rest of the user's known credentials remain tracked.
     */
    public suspend fun remove(context: Context, credentialIdBase64: String, rpId: String) {
        val current = read(context, rpId).toMutableList()
        if (!current.remove(credentialIdBase64)) return
        if (current.isEmpty()) {
            // Avoid persisting an empty array; clear() drops both
            // backing entries so subsequent add() takes the insert
            // path rather than overwriting an empty blob.
            clear(context, rpId)
        } else {
            val encoded = encodeList(current)
            writeToBlockStore(context, rpId, encoded)
            writeToEsp(context, rpId, encoded)
        }
    }

    // ------------------------------------------------------------------
    // Block Store
    // ------------------------------------------------------------------

    private fun blockStoreKey(rpId: String): String = "$BLOCKSTORE_KEY_PREFIX$rpId"

    private suspend fun readFromBlockStore(context: Context, rpId: String): List<String>? {
        val client = blockStoreClient(context) ?: return null
        return try {
            val request = RetrieveBytesRequest.Builder()
                .setKeys(listOf(blockStoreKey(rpId)))
                .build()
            val response = client.retrieveBytes(request).await()
            val bytes = response.blockstoreDataMap[blockStoreKey(rpId)]?.bytes
                ?: return emptyList()
            decodeList(String(bytes, Charsets.UTF_8))
        } catch (e: Exception) {
            Log.w(TAG, "Block Store retrieve failed for rpId=$rpId: ${e.message}")
            null
        }
    }

    private suspend fun writeToBlockStore(context: Context, rpId: String, encoded: String) {
        val client = blockStoreClient(context) ?: return
        try {
            val data = StoreBytesData.Builder()
                .setKey(blockStoreKey(rpId))
                .setBytes(encoded.toByteArray(Charsets.UTF_8))
                // Opt into cross-device + cross-reinstall sync via the
                // user's Google account. Without this, Block Store keeps
                // the value only locally — equivalent to ESP fallback.
                .setShouldBackupToCloud(true)
                .build()
            client.storeBytes(data).await()
        } catch (e: Exception) {
            // Best-effort: a Block Store write failure leaves us in
            // local-only mode. ESP write below still succeeds and the
            // next add() retries the cloud path.
            Log.w(TAG, "Block Store write failed for rpId=$rpId: ${e.message}")
        }
    }

    private suspend fun clearBlockStore(context: Context, rpId: String) {
        val client = blockStoreClient(context) ?: return
        try {
            val request = DeleteBytesRequest.Builder()
                .setKeys(listOf(blockStoreKey(rpId)))
                .build()
            client.deleteBytes(request).await()
        } catch (e: Exception) {
            Log.w(TAG, "Block Store delete failed for rpId=$rpId: ${e.message}")
        }
    }

    private fun blockStoreClient(context: Context): BlockstoreClient? = try {
        Blockstore.getClient(context.applicationContext)
    } catch (e: Exception) {
        // No Play Services / unsupported device. Fall back to ESP only.
        Log.w(TAG, "Block Store unavailable: ${e.message}")
        null
    }

    // ------------------------------------------------------------------
    // EncryptedSharedPreferences
    // ------------------------------------------------------------------

    private fun espKey(rpId: String): String = "$ESP_KEY_PREFIX$rpId"

    private fun readFromEsp(context: Context, rpId: String): List<String> {
        return try {
            val esp = encryptedPrefs(context)
            val raw = esp.getString(espKey(rpId), null) ?: return emptyList()
            decodeList(raw)
        } catch (e: Exception) {
            Log.w(TAG, "ESP read failed for rpId=$rpId: ${e.message}")
            emptyList()
        }
    }

    private fun writeToEsp(context: Context, rpId: String, encoded: String) {
        try {
            val esp = encryptedPrefs(context)
            esp.edit().putString(espKey(rpId), encoded).apply()
        } catch (e: Exception) {
            Log.w(TAG, "ESP write failed for rpId=$rpId: ${e.message}")
        }
    }

    private fun clearEsp(context: Context, rpId: String) {
        try {
            val esp = encryptedPrefs(context)
            esp.edit().remove(espKey(rpId)).apply()
        } catch (e: Exception) {
            Log.w(TAG, "ESP delete failed for rpId=$rpId: ${e.message}")
        }
    }

    private fun encryptedPrefs(context: Context) =
        EncryptedSharedPreferences.create(
            context.applicationContext,
            ESP_FILE,
            MasterKey.Builder(context.applicationContext)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build(),
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )

    // ------------------------------------------------------------------
    // Encoding
    // ------------------------------------------------------------------

    private fun encodeList(ids: List<String>): String {
        val arr = JSONArray()
        for (id in ids) arr.put(id)
        return arr.toString()
    }

    private fun decodeList(raw: String): List<String> = try {
        val arr = JSONArray(raw)
        val out = ArrayList<String>(arr.length())
        for (i in 0 until arr.length()) {
            val v = arr.optString(i, "")
            if (v.isNotEmpty()) out.add(v)
        }
        out
    } catch (e: Exception) {
        // Corrupt or stale shape. Surface as empty rather than crash —
        // the next successful create / sign-in will overwrite it.
        emptyList()
    }
}
