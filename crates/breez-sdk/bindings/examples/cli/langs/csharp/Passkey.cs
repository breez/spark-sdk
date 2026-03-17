using System.Security.Cryptography;
using Breez.Sdk.Spark;

namespace BreezCli;

// ---------------------------------------------------------------------------
// Passkey provider types
// ---------------------------------------------------------------------------

/// <summary>
/// Identifies which PRF provider to use.
/// </summary>
public enum PasskeyProvider
{
    File,
    YubiKey,
    Fido2,
}

/// <summary>
/// Configuration for passkey seed derivation.
/// </summary>
public class PasskeyConfig
{
    /// <summary>The PRF provider to use.</summary>
    public PasskeyProvider Provider { get; init; }
    /// <summary>Optional label for seed derivation. If omitted, the core uses the default name.</summary>
    public string? Label { get; init; }
    /// <summary>Whether to list and select from labels published to Nostr.</summary>
    public bool ListLabels { get; init; }
    /// <summary>Whether to publish the label to Nostr.</summary>
    public bool StoreLabel { get; init; }
    /// <summary>Optional relying party ID for FIDO2 provider (default: keys.breez.technology).</summary>
    public string? RpId { get; init; }
}

// ---------------------------------------------------------------------------
// PasskeyProvider helpers
// ---------------------------------------------------------------------------

public static class PasskeyProviderExtensions
{
    /// <summary>
    /// Parses a provider name string into a PasskeyProvider.
    /// </summary>
    public static PasskeyProvider ParseProvider(string s)
    {
        return s.ToLower() switch
        {
            "file" => PasskeyProvider.File,
            "yubikey" => PasskeyProvider.YubiKey,
            "fido2" => PasskeyProvider.Fido2,
            _ => throw new ArgumentException(
                $"Invalid passkey provider '{s}' (valid: file, yubikey, fido2)")
        };
    }

    /// <summary>
    /// Creates a PasskeyPrfProvider for the given provider type.
    /// </summary>
    public static PasskeyPrfProvider BuildPrfProvider(
        PasskeyProvider provider,
        string dataDir,
        string? rpId = null)
    {
        return provider switch
        {
            PasskeyProvider.File => new FilePrfProvider(dataDir),
            PasskeyProvider.YubiKey => new NotYetSupportedProvider("YubiKey"),
            PasskeyProvider.Fido2 => new NotYetSupportedProvider("FIDO2"),
            _ => throw new ArgumentException($"Unknown passkey provider: {provider}")
        };
    }
}

// ---------------------------------------------------------------------------
// File-based PRF provider
// ---------------------------------------------------------------------------

/// <summary>
/// File-based implementation of PasskeyPrfProvider.
///
/// Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
/// randomly on first use and persisted to disk.
///
/// Security Notes:
/// - The secret file should be protected with appropriate file permissions
/// - This is less secure than hardware-backed solutions like YubiKey
/// - Suitable for development/testing or when hardware keys are unavailable
/// </summary>
public class FilePrfProvider : PasskeyPrfProvider
{
    private const string SecretFileName = "seedless-restore-secret";
    private readonly byte[] _secret;

    /// <summary>
    /// Create a new FilePrfProvider using a secret from the specified data directory.
    /// If the secret file doesn't exist, a random 32-byte secret is generated and saved.
    /// </summary>
    public FilePrfProvider(string dataDir)
    {
        var secretPath = Path.Combine(dataDir, SecretFileName);

        if (File.Exists(secretPath))
        {
            var bytes = File.ReadAllBytes(secretPath);
            if (bytes.Length != 32)
            {
                throw new InvalidOperationException(
                    $"Invalid secret file: expected 32 bytes, got {bytes.Length}");
            }
            _secret = bytes;
        }
        else
        {
            // Generate new random secret
            _secret = new byte[32];
            RandomNumberGenerator.Fill(_secret);

            // Ensure data directory exists
            Directory.CreateDirectory(dataDir);

            // Save secret to file
            File.WriteAllBytes(secretPath, _secret);
        }
    }

    public async Task<byte[]> DerivePrfSeed(string salt)
    {
        using var hmac = new HMACSHA256(_secret);
        var result = hmac.ComputeHash(System.Text.Encoding.UTF8.GetBytes(salt));
        return await Task.FromResult(result);
    }

    public async Task<bool> IsPrfAvailable()
    {
        // File-based PRF is always available once initialized
        return await Task.FromResult(true);
    }
}

// ---------------------------------------------------------------------------
// Stub provider for hardware-dependent backends
// ---------------------------------------------------------------------------

/// <summary>
/// Stub provider for backends that are not yet supported in the C# CLI.
/// </summary>
public class NotYetSupportedProvider : PasskeyPrfProvider
{
    private readonly string _name;

    public NotYetSupportedProvider(string name)
    {
        _name = name;
    }

    public Task<byte[]> DerivePrfSeed(string salt)
    {
        throw new NotSupportedException(
            $"{_name} passkey provider is not yet supported in the C# CLI");
    }

    public Task<bool> IsPrfAvailable()
    {
        throw new NotSupportedException(
            $"{_name} passkey provider is not yet supported in the C# CLI");
    }
}

// ---------------------------------------------------------------------------
// Passkey seed resolution (orchestration)
// ---------------------------------------------------------------------------

public static class PasskeyResolver
{
    /// <summary>
    /// Derives a wallet seed using the given PRF provider,
    /// matching the Rust CLI's resolve_passkey_seed logic.
    /// </summary>
    public static async Task<Seed> ResolvePasskeySeed(
        PasskeyPrfProvider provider,
        string? breezApiKey,
        string? label,
        bool listLabels,
        bool storeLabel)
    {
        var relayConfig = new NostrRelayConfig(
            breezApiKey: breezApiKey
        );
        var passkey = new Passkey(provider, relayConfig);

        // --store-label: publish to Nostr
        if (storeLabel && label != null)
        {
            Console.WriteLine($"Publishing label '{label}' to Nostr...");
            await passkey.StoreLabel(label);
            Console.WriteLine($"Label '{label}' published successfully.");
        }

        // --list-labels: query Nostr and prompt user to select
        string? resolvedName = label;
        if (listLabels)
        {
            Console.WriteLine("Querying Nostr for available labels...");
            var labels = await passkey.ListLabels();

            if (labels.Length == 0)
            {
                throw new InvalidOperationException(
                    "No labels found on Nostr for this identity");
            }

            Console.WriteLine("Available labels:");
            for (int i = 0; i < labels.Length; i++)
            {
                Console.WriteLine($"  {i + 1}: {labels[i]}");
            }

            Console.Write($"Select label (1-{labels.Length}): ");
            Console.Out.Flush();
            var input = Console.ReadLine();
            if (!int.TryParse(input?.Trim(), out int idx))
            {
                throw new InvalidOperationException("Invalid selection");
            }

            if (idx < 1 || idx > labels.Length)
            {
                throw new InvalidOperationException("Selection out of range");
            }

            resolvedName = labels[idx - 1];
        }

        var wallet = await passkey.GetWallet(label: resolvedName);
        return wallet.seed;
    }
}
