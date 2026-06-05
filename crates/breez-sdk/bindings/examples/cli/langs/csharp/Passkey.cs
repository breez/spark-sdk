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
public class CliPasskeyConfig
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
    /// Creates a PrfProvider for the given provider type.
    /// </summary>
    public static PrfProvider BuildPrfProvider(
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
/// File-based implementation of PrfProvider.
///
/// Uses HMAC-SHA256 with a secret stored in a file. The secret is generated
/// randomly on first use and persisted to disk.
///
/// Security Notes:
/// - The secret file should be protected with appropriate file permissions
/// - This is less secure than hardware-backed solutions like YubiKey
/// - Suitable for development/testing or when hardware keys are unavailable
/// </summary>
public class FilePrfProvider : PrfProvider
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

    public Task<DeriveSeedsOutput> DeriveSeeds(DeriveSeedsRequest request)
    {
        using var hmac = new HMACSHA256(_secret);
        var seeds = new byte[request.salts.Length][];
        for (int i = 0; i < request.salts.Length; i++)
        {
            seeds[i] = hmac.ComputeHash(System.Text.Encoding.UTF8.GetBytes(request.salts[i]));
        }
        return Task.FromResult(new DeriveSeedsOutput(seeds, credentialId: null));
    }

    public Task<bool> IsSupported() => Task.FromResult(true);

    public Task<PasskeyCredential> CreatePasskey(byte[][] excludeCredentials)
    {
        throw new NotSupportedException(
            "File-backed PRF provider does not implement create-credential; " +
            "use sign-in by label instead.");
    }

    public Task<DomainAssociation> CheckDomainAssociation() =>
        Task.FromResult<DomainAssociation>(
            new DomainAssociation.Skipped("FilePrfProvider does not verify domain association"));
}

// ---------------------------------------------------------------------------
// Stub provider for hardware-dependent backends
// ---------------------------------------------------------------------------

/// <summary>
/// Stub provider for backends that are not yet supported in the C# CLI.
/// </summary>
public class NotYetSupportedProvider : PrfProvider
{
    private readonly string _name;

    public NotYetSupportedProvider(string name)
    {
        _name = name;
    }

    private NotSupportedException NotYet() =>
        new($"{_name} passkey provider is not yet supported in the C# CLI");

    public Task<DeriveSeedsOutput> DeriveSeeds(DeriveSeedsRequest request) => throw NotYet();

    public Task<bool> IsSupported() => throw NotYet();

    public Task<PasskeyCredential> CreatePasskey(byte[][] excludeCredentials) => throw NotYet();

    public Task<DomainAssociation> CheckDomainAssociation() =>
        Task.FromResult<DomainAssociation>(
            new DomainAssociation.Skipped($"{_name} does not verify domain association"));
}

// ---------------------------------------------------------------------------
// Passkey seed resolution (orchestration)
// ---------------------------------------------------------------------------

public static class PasskeyResolver
{
    /// <summary>
    /// Derives a wallet seed using the given PRF provider, matching the Rust
    /// CLI's resolve_passkey_seed logic.
    /// </summary>
    public static async Task<Seed> ResolvePasskeySeed(
        PrfProvider provider,
        string? breezApiKey,
        string? label,
        bool listLabels,
        bool storeLabel)
    {
        var passkey = new PasskeyClient(provider, breezApiKey, config: null);

        // --list-labels: discovery sign-in (null label) returns the
        // discovered label set; prompt user to pick.
        string? resolvedName = label;
        if (listLabels)
        {
            Console.WriteLine("Querying Nostr for available labels...");
            var discoveryResponse = await passkey.SignIn(new SignInRequest(label: null));

            if (discoveryResponse.labels == null || discoveryResponse.labels.Length == 0)
            {
                throw new InvalidOperationException(
                    "No labels found on Nostr for this identity");
            }

            Console.WriteLine("Available labels:");
            for (int i = 0; i < discoveryResponse.labels.Length; i++)
            {
                Console.WriteLine($"  {i + 1}: {discoveryResponse.labels[i]}");
            }

            Console.Write($"Select label (1-{discoveryResponse.labels.Length}): ");
            Console.Out.Flush();
            var input = Console.ReadLine();
            if (!int.TryParse(input?.Trim(), out int idx))
            {
                throw new InvalidOperationException("Invalid selection");
            }

            if (idx < 1 || idx > discoveryResponse.labels.Length)
            {
                throw new InvalidOperationException("Selection out of range");
            }

            resolvedName = discoveryResponse.labels[idx - 1];
        }

        // --store-label: publish before final sign-in so a fresh client
        // can discover the label later.
        if (storeLabel && resolvedName != null)
        {
            Console.WriteLine($"Publishing label '{resolvedName}' to Nostr...");
            await passkey.Labels().Store(resolvedName);
            Console.WriteLine($"Label '{resolvedName}' published successfully.");
        }

        var response = await passkey.SignIn(new SignInRequest(label: resolvedName));
        return response.wallet.seed;
    }
}
