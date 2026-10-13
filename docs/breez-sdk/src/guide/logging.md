# Adding logging

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

{{#tabs getting_started:logging}}

When several SDK instances run in the same process, for example in a [multi-user configuration](./server_mode.md), each log entry's {{#name instance_id}} holds the identity public key of the wallet whose instance emitted it, so you can tell their logs apart. Entries not emitted by an instance leave it unset.
