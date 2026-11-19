// dart:mirrors is available at runtime (JIT via `dart run`) but not recognized
// by `dart analyze` when Flutter packages are in the dependency tree.
// ignore_for_file: uri_does_not_exist, undefined_function, undefined_class
// ignore_for_file: type_test_with_undefined_name, undefined_identifier
import 'dart:convert';
import 'dart:mirrors';

/// Converts a generated binding object to the JSON shape the Rust CLI prints
/// with serde: snake_case keys, and enum variants externally tagged by their
/// Rust name (`{"Bolt11Invoice": {...}}`, or `"SparkAddress"` for a variant
/// without fields).
Object? objToSerializable(Object? obj) {
  if (obj == null) return null;
  if (obj is String || obj is int || obj is double || obj is bool) return obj;
  if (obj is BigInt) return obj.isValidInt ? obj.toInt() : obj.toString();
  if (obj is Enum) return _pascalCase(obj.name);
  if (obj is List) return obj.map(objToSerializable).toList();
  if (obj is Map) {
    return obj.map((k, v) => MapEntry(k.toString(), objToSerializable(v)));
  }

  final mirror = reflect(obj);
  final fields = _reflectFields(mirror);
  final variant = _variantName(mirror.type);
  if (variant == null) return fields;
  if (fields.isEmpty) return variant;
  if (fields.length == 1 && fields.containsKey('v0')) return {variant: fields['v0']};
  if (fields.containsKey('v0')) return {variant: fields.values.toList()};
  return {variant: fields};
}

/// The Rust variant name of a generated enum variant class, which uniffi-dart
/// names `<Variant><Enum>` (`Bolt11InvoiceInputType`), with "Error" in error
/// names renamed to "Exception". `null` for a class that is not a variant.
String? _variantName(ClassMirror type) {
  final parent = type.superclass;
  if (parent == null || !parent.isAbstract || parent.reflectedType == Object) return null;
  final name = MirrorSystem.getName(type.simpleName);
  final parentName = MirrorSystem.getName(parent.simpleName);
  if (!name.endsWith(parentName) || name == parentName) return null;
  final variant = name.substring(0, name.length - parentName.length);
  return parentName.endsWith('Exception') ? variant.replaceAll('Exception', 'Error') : variant;
}

/// Walk the class hierarchy and collect all public, non-static fields.
Map<String, dynamic> _reflectFields(InstanceMirror mirror) {
  final fields = <String, dynamic>{};
  ClassMirror? current = mirror.type;

  while (current != null && current.reflectedType != Object) {
    for (final entry in current.declarations.entries) {
      final decl = entry.value;
      if (decl is VariableMirror && !decl.isStatic && !decl.isPrivate) {
        final name = _snakeCase(MirrorSystem.getName(entry.key));
        if (!fields.containsKey(name)) {
          fields[name] = objToSerializable(mirror.getField(entry.key).reflectee);
        }
      }
    }
    current = current.superclass;
  }

  return fields;
}

String _snakeCase(String name) => name.replaceAllMapped(RegExp('[A-Z]'), (m) => '_${m[0]!.toLowerCase()}');

String _pascalCase(String name) => name.isEmpty ? name : name[0].toUpperCase() + name.substring(1);

/// Serialize an object to a pretty-printed JSON string.
String serialize(Object? obj) {
  return const JsonEncoder.withIndent('  ').convert(objToSerializable(obj));
}

/// Serialize an object to single-line JSON.
String serializeCompact(Object? obj) => jsonEncode(objToSerializable(obj));

/// Print an object as formatted JSON.
void printValue(Object? obj) {
  print(serialize(obj));
}
