import Foundation
import BigNumber

// MARK: - Reflection-based JSON serialization for UniFFI types

/// Recursively converts any SDK object to a JSON-serializable representation
/// using Swift's Mirror reflection. Handles structs, enums with associated
/// values, optionals, arrays, dictionaries, BInt, and Data/[UInt8].
func objToSerializable(_ value: Any?) -> Any? {
    guard let value = value else { return nil }

    // Unwrap Optional
    let mirror = Mirror(reflecting: value)
    if mirror.displayStyle == .optional {
        if mirror.children.isEmpty {
            return nil
        }
        return objToSerializable(mirror.children.first!.value)
    }

    // Primitives
    if let s = value as? String { return s }
    if let b = value as? Bool { return b }
    if let n = value as? Int { return n }
    if let n = value as? Int8 { return Int(n) }
    if let n = value as? Int16 { return Int(n) }
    if let n = value as? Int32 { return Int(n) }
    if let n = value as? Int64 { return n }
    if let n = value as? UInt { return n }
    if let n = value as? UInt8 { return Int(n) }
    if let n = value as? UInt16 { return Int(n) }
    if let n = value as? UInt32 { return n }
    if let n = value as? UInt64 { return n }
    if let n = value as? Float { return n }
    if let n = value as? Double { return n }

    // BInt → string
    if let big = value as? BInt { return String(big) }

    // Data → hex string
    if let data = value as? Data {
        return data.map { String(format: "%02x", $0) }.joined()
    }

    // [UInt8] → hex string
    if let bytes = value as? [UInt8] {
        return bytes.map { String(format: "%02x", $0) }.joined()
    }

    // Array
    if let arr = value as? [Any] {
        return arr.map { objToSerializable($0) }
    }

    // Dictionary
    if mirror.displayStyle == .dictionary {
        var result: [String: Any?] = [:]
        for child in mirror.children {
            let pair = Mirror(reflecting: child.value)
            let children = Array(pair.children)
            if children.count == 2 {
                let key = "\(children[0].value)"
                result[key] = objToSerializable(children[1].value)
            }
        }
        return result
    }

    // Enum with associated values
    if mirror.displayStyle == .enum {
        let caseName = String(describing: value).components(separatedBy: "(").first ?? String(describing: value)

        if mirror.children.isEmpty {
            // Unit variant
            return caseName
        }

        var result: [String: Any?] = ["type": caseName]
        for child in mirror.children {
            if let label = child.label {
                let key = camelToSnakeCase(label)
                result[key] = objToSerializable(child.value)
            } else {
                // Positional associated value — try to serialize it as the main content
                let inner = objToSerializable(child.value)
                if let dict = inner as? [String: Any?] {
                    for (k, v) in dict { result[k] = v }
                } else {
                    result["value"] = inner
                }
            }
        }

        // If there's only a type and one other field, and this looks like a wrapper,
        // keep the structure for clarity
        return result
    }

    // Struct / class — enumerate fields via Mirror
    if mirror.displayStyle == .struct || mirror.displayStyle == .class || mirror.displayStyle == nil {
        if mirror.children.isEmpty {
            return String(describing: value)
        }
        var result: [String: Any?] = [:]
        for child in mirror.children {
            if let label = child.label {
                let key = camelToSnakeCase(label)
                result[key] = objToSerializable(child.value)
            }
        }
        if result.isEmpty {
            return String(describing: value)
        }
        return result
    }

    // Tuple
    if mirror.displayStyle == .tuple {
        var result: [String: Any?] = [:]
        for child in mirror.children {
            if let label = child.label {
                result[label] = objToSerializable(child.value)
            }
        }
        return result
    }

    return String(describing: value)
}

/// Converts a camelCase string to snake_case.
func camelToSnakeCase(_ input: String) -> String {
    var result = ""
    for (i, char) in input.enumerated() {
        if char.isUppercase {
            if i > 0 {
                result += "_"
            }
            result += char.lowercased()
        } else {
            result += String(char)
        }
    }
    return result
}

/// Serializes any SDK object to a pretty-printed JSON string.
func serialize(_ value: Any?) -> String {
    guard let serializable = objToSerializable(value) else { return "null" }
    // JSONSerialization requires a top-level array or dictionary.
    // Wrap bare primitives (String, Number, Bool) so they round-trip safely.
    if !JSONSerialization.isValidJSONObject(serializable) {
        return "\(serializable)"
    }
    do {
        let jsonData = try JSONSerialization.data(
            withJSONObject: serializable,
            options: [.prettyPrinted, .sortedKeys]
        )
        return String(data: jsonData, encoding: .utf8) ?? String(describing: value)
    } catch {
        return String(describing: value)
    }
}

/// Prints any SDK object as pretty JSON to stdout.
func printValue(_ value: Any?) {
    print(serialize(value))
}
