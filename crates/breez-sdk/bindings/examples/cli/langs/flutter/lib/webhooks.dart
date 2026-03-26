import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'serialization.dart';

/// Webhook subcommand names (used for tab completion).
const webhookCommandNames = ['webhooks register', 'webhooks unregister', 'webhooks list'];

typedef WebhookHandler = Future<void> Function(BreezSdk sdk, List<String> args);

class _WebhookEntry {
  final String description;
  final WebhookHandler handler;
  const _WebhookEntry(this.description, this.handler);
}

Map<String, _WebhookEntry>? _registry;

Map<String, _WebhookEntry> _getRegistry() {
  return _registry ??= {
    'register': _WebhookEntry('Register a new webhook', _handleRegister),
    'unregister': _WebhookEntry('Unregister a webhook', _handleUnregister),
    'list': _WebhookEntry('List all registered webhooks', _handleList),
  };
}

/// Dispatch a webhook subcommand given the args after 'webhooks'.
Future<void> dispatchWebhookCommand(List<String> args, BreezSdk sdk) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help' || args[0] == '--help') {
    print('\nWebhook subcommands:\n');
    for (final entry in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  webhooks ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print("Unknown webhook subcommand: $subName. Use 'webhooks help' for available commands.");
    return;
  }

  await registry[subName]!.handler(sdk, subArgs);
}

WebhookEventType? _parseEventType(String s) {
  switch (s) {
    case 'lightning-receive':
      return WebhookEventType.lightningReceiveFinished();
    case 'lightning-send':
      return WebhookEventType.lightningSendFinished();
    case 'coop-exit':
      return WebhookEventType.coopExitFinished();
    case 'static-deposit':
      return WebhookEventType.staticDepositFinished();
    default:
      return null;
  }
}

// --- register ---

Future<void> _handleRegister(BreezSdk sdk, List<String> args) async {
  if (args.length < 3 || args.first == 'help' || args.first == '--help') {
    print('Usage: webhooks register <url> <secret> <event_type> [event_type...]');
    print('Event types: lightning-receive, lightning-send, coop-exit, static-deposit');
    return;
  }
  final url = args[0];
  final secret = args[1];
  final eventStrings = args.sublist(2);

  final eventTypes = <WebhookEventType>[];
  for (final e in eventStrings) {
    final eventType = _parseEventType(e);
    if (eventType == null) {
      print(
        'Unknown event type: $e. '
        'Valid values: lightning-receive, lightning-send, coop-exit, static-deposit',
      );
      return;
    }
    eventTypes.add(eventType);
  }

  final result = await sdk.registerWebhook(
    request: RegisterWebhookRequest(url: url, secret: secret, eventTypes: eventTypes),
  );
  printValue(result);
}

// --- unregister ---

Future<void> _handleUnregister(BreezSdk sdk, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: webhooks unregister <webhook_id>');
    return;
  }
  final webhookId = args[0];
  await sdk.unregisterWebhook(request: UnregisterWebhookRequest(webhookId: webhookId));
  print('Webhook unregistered successfully');
}

// --- list ---

Future<void> _handleList(BreezSdk sdk, List<String> args) async {
  final result = await sdk.listWebhooks();
  printValue(result);
}
