# Adding logging

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

{{#tabs getting_started:logging}}

<div class="warning">
<h4>Developer note</h4>

If your logger can throw, catch errors inside it. The SDK calls {{#name log}} from its own threads and has no way to report an exception that escapes it, so it panics, which can stop the app.
</div>

Calling {{#name init_logging}} again replaces the logger and the log filter, and the log directory where your language takes one. A Flutter app can call it at every startup, so logging continues after a hot restart, or when Android reopens the app in the same process.

The logger is shared by the whole process. If a second Flutter engine, such as one running a background message handler, calls {{#name init_logging}}, its logger receives every entry from then on, including those of the app's own SDK instance. The app's logger receives none until the app calls {{#name init_logging}} again.

In JavaScript, {{#name ConsoleLogger}} is a logger that prints each log entry to the console: errors with `console.error`, warnings with `console.warn`, info entries with `console.info`, and debug and trace entries with `console.debug`.

When several SDK instances run in the same process, for example in a [multi-user configuration](./server_mode.md), each log entry's {{#name instance_id}} holds the identity public key of the wallet whose instance emitted it, so you can tell their logs apart. Entries not emitted by an instance leave it unset.
