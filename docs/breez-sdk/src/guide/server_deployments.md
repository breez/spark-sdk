# Server deployments

The SDK runs on a server in two deployments that have little in common beyond the machine they share. They pull in opposite directions, so pick the one that matches yours before taking any configuration advice.

| | [Serving end-user wallets](server_mode.md) | [Treasury wallets](treasury.md) |
|---|---|---|
| Wallets per process | many | one |
| SDK lifetime | built per request, disconnected when it returns | one instance, long lived |
| Whose funds | your users' | your own |
| Background tasks | off: your host drives sync, claiming and event delivery | on, tuned for an unattended wallet |
| Config preset | {{#name default_server_config}} | {{#name default_config}} |

**[Serving end-user wallets](server_mode.md)** covers a service that holds a wallet per user. The SDK is treated as a library: an instance is built for one operation and thrown away, and your infrastructure decides when anything happens. Most of that page is about what you now have to drive yourself.

**[Treasury wallets](treasury.md)** covers a single wallet your own service owns, such as a float or a settlement account. That is an ordinary SDK deployment and everything in the rest of this guide applies to it unchanged. The page is short because only a few defaults are worth revisiting.

A service can run both: user wallets on one profile and its own float on the other. They are separate SDK instances with separate configs, so follow each page for its own part rather than trying to find a setting that suits both.
