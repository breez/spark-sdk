using System.Numerics;
using System.Reflection;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace BreezCli;

/// <summary>
/// Reflection-based JSON serialization for UniFFI-generated C# types.
/// UniFFI types use subclass patterns for enums (e.g., SdkEvent.Synced) and
/// may not have System.Text.Json attributes, so we use reflection to convert
/// them into JSON-serializable dictionaries.
/// </summary>
public static class Serialization
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
    };

    /// <summary>
    /// Serializes a value to a pretty-printed JSON string.
    /// </summary>
    public static string Serialize(object? value)
    {
        var converted = ObjToJsonElement(value);
        return JsonSerializer.Serialize(converted, JsonOptions);
    }

    /// <summary>
    /// Prints a value as pretty JSON to stdout.
    /// </summary>
    public static void PrintValue(object? value)
    {
        Console.WriteLine(Serialize(value));
    }

    /// <summary>
    /// Known enum/tagged-union base type prefixes used to detect variant types
    /// and extract their variant name.
    /// </summary>
    private static readonly string[] VariantPrefixes =
    {
        "SendPaymentOptions+", "SendPaymentMethod+",
        "ReceivePaymentMethod+", "PaymentDetailsFilter+", "PaymentDetails+",
        "LnurlCallbackStatus+", "OnchainConfirmationSpeed+",
        "ConversionType+", "TokenTransactionType+",
        "SparkHtlcStatus+", "PaymentStatus+", "PaymentType+",
        "ServiceStatus+", "SdkEvent+", "InputType+",
        "AssetFilter+", "FeePolicy+", "MaxFee+", "Fee+", "Seed+",
    };

    /// <summary>
    /// Recursively converts a C# object into a JSON-serializable structure
    /// (dictionaries, lists, primitives) using reflection.
    /// </summary>
    private static object? ObjToJsonElement(object? value)
    {
        if (value == null) return null;

        var type = value.GetType();

        // Primitives
        if (type == typeof(string)) return value;
        if (type == typeof(bool)) return value;
        if (type == typeof(byte)) return value;
        if (type == typeof(short)) return value;
        if (type == typeof(int)) return value;
        if (type == typeof(long)) return value;
        if (type == typeof(ushort)) return value;
        if (type == typeof(uint)) return value;
        if (type == typeof(ulong)) return value;
        if (type == typeof(float)) return value;
        if (type == typeof(double)) return value;
        if (type == typeof(decimal)) return value;

        // BigInteger -> string
        if (type == typeof(BigInteger))
        {
            return ((BigInteger)value).ToString();
        }

        // Nullable value types
        if (type.IsGenericType && type.GetGenericTypeDefinition() == typeof(Nullable<>))
        {
            var underlyingValue = type.GetProperty("Value")!.GetValue(value);
            return ObjToJsonElement(underlyingValue);
        }

        // byte[] -> hex string
        if (type == typeof(byte[]))
        {
            return Convert.ToHexString((byte[])value).ToLowerInvariant();
        }

        // Enums -> string
        if (type.IsEnum)
        {
            return value.ToString();
        }

        // Dictionary<K,V>
        if (type.IsGenericType && type.GetGenericTypeDefinition() == typeof(Dictionary<,>))
        {
            var dict = new Dictionary<string, object?>();
            var enumerable = (System.Collections.IEnumerable)value;
            foreach (var item in enumerable)
            {
                var keyProp = item.GetType().GetProperty("Key")!;
                var valProp = item.GetType().GetProperty("Value")!;
                var key = keyProp.GetValue(item)?.ToString() ?? "";
                dict[key] = ObjToJsonElement(valProp.GetValue(item));
            }
            return dict;
        }

        // IList<T> / arrays
        if (type.IsArray || (type.IsGenericType &&
            type.GetInterfaces().Any(i =>
                i.IsGenericType && i.GetGenericTypeDefinition() == typeof(IList<>))))
        {
            var list = new List<object?>();
            foreach (var item in (System.Collections.IEnumerable)value)
            {
                list.Add(ObjToJsonElement(item));
            }
            return list;
        }

        // Structs and classes (including UniFFI record types and tagged union variants)
        if (type.IsClass || type.IsValueType)
        {
            var dict = new Dictionary<string, object?>();

            // Detect tagged union variant types (nested classes like SdkEvent.Synced)
            var fullName = type.FullName ?? type.Name;
            var variantName = ExtractVariantName(fullName, type.Name);
            if (variantName != null)
            {
                dict["type"] = variantName;
            }

            // Get all public instance fields (UniFFI uses public fields)
            var fields = type.GetFields(BindingFlags.Public | BindingFlags.Instance);
            foreach (var field in fields)
            {
                var name = ToSnakeCase(field.Name);
                var fieldValue = field.GetValue(value);
                dict[name] = ObjToJsonElement(fieldValue);
            }

            // Also get public instance properties (some UniFFI bindings use properties)
            var properties = type.GetProperties(BindingFlags.Public | BindingFlags.Instance);
            foreach (var prop in properties)
            {
                // Skip indexers and properties we've already covered via fields
                if (prop.GetIndexParameters().Length > 0) continue;
                var name = ToSnakeCase(prop.Name);
                if (dict.ContainsKey(name)) continue;

                try
                {
                    var propValue = prop.GetValue(value);
                    dict[name] = ObjToJsonElement(propValue);
                }
                catch
                {
                    // Skip properties that throw on access
                }
            }

            return dict;
        }

        return value.ToString();
    }

    /// <summary>
    /// Checks if a type is a tagged union variant and extracts the variant name.
    /// For nested classes like "Breez.Sdk.Spark.SdkEvent+Synced", returns "Synced".
    /// </summary>
    private static string? ExtractVariantName(string fullName, string shortName)
    {
        // Check for nested type pattern (Parent+Child)
        foreach (var prefix in VariantPrefixes)
        {
            // Check against the short type name pattern
            if (shortName.Contains('+'))
            {
                var parts = shortName.Split('+');
                if (parts.Length >= 2)
                {
                    return parts[^1];
                }
            }
        }

        // Check the full name for the nested type pattern
        if (fullName.Contains('+'))
        {
            var parts = fullName.Split('+');
            if (parts.Length >= 2)
            {
                // Verify the parent is one of our known enum types
                var parentName = parts[^2].Split('.').Last();
                foreach (var prefix in VariantPrefixes)
                {
                    var prefixBase = prefix.TrimEnd('+');
                    if (parentName == prefixBase)
                    {
                        return parts[^1];
                    }
                }
            }
        }

        return null;
    }

    /// <summary>
    /// Converts a PascalCase or camelCase string to snake_case.
    /// </summary>
    private static string ToSnakeCase(string input)
    {
        if (string.IsNullOrEmpty(input)) return input;

        var sb = new StringBuilder();
        for (int i = 0; i < input.Length; i++)
        {
            char c = input[i];
            if (char.IsUpper(c))
            {
                if (i > 0)
                {
                    // Don't add underscore between consecutive uppercase (like "URL" -> "url")
                    bool prevIsUpper = char.IsUpper(input[i - 1]);
                    bool nextIsLower = i + 1 < input.Length && char.IsLower(input[i + 1]);

                    if (!prevIsUpper || nextIsLower)
                    {
                        sb.Append('_');
                    }
                }
                sb.Append(char.ToLower(c));
            }
            else
            {
                sb.Append(c);
            }
        }
        return sb.ToString();
    }
}
