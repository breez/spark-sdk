import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

/// Contacts subcommand names (used for tab completion).
const contactsCommandNames = ['contacts add', 'contacts update', 'contacts delete', 'contacts list'];

typedef ContactsHandler = Future<void> Function(BreezSdk sdk, List<String> args);

class _ContactsEntry {
  final String description;
  final ContactsHandler handler;
  const _ContactsEntry(this.description, this.handler);
}

Map<String, _ContactsEntry>? _registry;

Map<String, _ContactsEntry> _getRegistry() {
  return _registry ??= {
    'add': _ContactsEntry('Add a new contact', _handleAdd),
    'update': _ContactsEntry('Update an existing contact', _handleUpdate),
    'delete': _ContactsEntry('Delete a contact', _handleDelete),
    'list': _ContactsEntry('List contacts', _handleList),
  };
}

/// Dispatch a contacts subcommand given the args after 'contacts'.
Future<void> dispatchContactsCommand(List<String> args, BreezSdk sdk) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help' || args[0] == '--help') {
    print('\nContacts subcommands (not yet supported in Flutter):\n');
    for (final entry in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  contacts ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print("Unknown contacts subcommand: $subName. Use 'contacts help' for available commands.");
    return;
  }

  await registry[subName]!.handler(sdk, subArgs);
}

// --- add ---

Future<void> _handleAdd(BreezSdk sdk, List<String> args) async {
  print('Not yet supported');
}

// --- update ---

Future<void> _handleUpdate(BreezSdk sdk, List<String> args) async {
  print('Not yet supported');
}

// --- delete ---

Future<void> _handleDelete(BreezSdk sdk, List<String> args) async {
  print('Not yet supported');
}

// --- list ---

Future<void> _handleList(BreezSdk sdk, List<String> args) async {
  print('Not yet supported');
}
