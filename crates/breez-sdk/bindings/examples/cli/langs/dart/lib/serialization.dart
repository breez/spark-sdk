import 'dart:convert';

/// Recursively convert an FRB-generated object to a JSON-serializable structure.
Object? objToSerializable(Object? obj) {
  if (obj == null) return null;
  if (obj is String || obj is int || obj is double || obj is bool) return obj;
  if (obj is BigInt) return obj.toString();
  if (obj is List) return obj.map(objToSerializable).toList();
  if (obj is Map) {
    return obj.map((k, v) => MapEntry(k.toString(), objToSerializable(v)));
  }

  // For FRB-generated sealed classes / freezed types, try to use toJson()
  // or toString() as fallback.
  try {
    // ignore: avoid_dynamic_calls
    final json = (obj as dynamic).toJson();
    if (json is Map) {
      return json.map((k, v) => MapEntry(k.toString(), objToSerializable(v)));
    }
    return json;
  } catch (_) {
    // No toJson() method available
  }

  return obj.toString();
}

/// Serialize an object to a pretty-printed JSON string.
String serialize(Object? obj) {
  return const JsonEncoder.withIndent('  ').convert(objToSerializable(obj));
}

/// Print an object as formatted JSON.
void printValue(Object? obj) {
  print(serialize(obj));
}
