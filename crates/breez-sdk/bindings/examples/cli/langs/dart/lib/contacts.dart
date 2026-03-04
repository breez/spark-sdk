import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'serialization.dart';

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
    print('\nContacts subcommands:\n');
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
  if (args.length < 2 || args.first == 'help' || args.first == '--help') {
    print('Usage: contacts add <name> <payment_identifier>');
    return;
  }
  final name = args[0];
  final paymentIdentifier = args[1];
  final result = await sdk.addContact(
    request: AddContactRequest(name: name, paymentIdentifier: paymentIdentifier),
  );
  printValue(result);
}

// --- update ---

Future<void> _handleUpdate(BreezSdk sdk, List<String> args) async {
  if (args.length < 3 || args.first == 'help' || args.first == '--help') {
    print('Usage: contacts update <id> <name> <payment_identifier>');
    return;
  }
  final id = args[0];
  final name = args[1];
  final paymentIdentifier = args[2];
  final result = await sdk.updateContact(
    request: UpdateContactRequest(id: id, name: name, paymentIdentifier: paymentIdentifier),
  );
  printValue(result);
}

// --- delete ---

Future<void> _handleDelete(BreezSdk sdk, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: contacts delete <id>');
    return;
  }
  final id = args[0];
  await sdk.deleteContact(id: id);
  print('Contact deleted successfully');
}

// --- list ---

Future<void> _handleList(BreezSdk sdk, List<String> args) async {
  int? offset;
  int? limit;
  if (args.isNotEmpty && args.first != 'help' && args.first != '--help') {
    offset = int.tryParse(args[0]);
    if (args.length > 1) {
      limit = int.tryParse(args[1]);
    }
  } else if (args.isNotEmpty) {
    print('Usage: contacts list [offset] [limit]');
    return;
  }
  final result = await sdk.listContacts(request: ListContactsRequest(offset: offset, limit: limit));
  printValue(result);
}
