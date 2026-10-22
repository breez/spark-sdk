# UX Guidelines

These guidelines describe how to build a UI/UX on top of the Breez SDK that feels natural to end users. They are design principles, not feature documentation: each page says what experience to create and why, and links to the feature guides for the how.

> **Reference:** These guidelines are implemented in **[Glow](https://glow-app.co)**. Use it as the primary UX reference during SDK implementation, and adapt the recommendations to your own use case.

## Core UX principles

- **Simplicity over choice**: users should not have to pick protocols or rails unless absolutely necessary. The wallet knows what a pasted string is and what to do with it.
- **Transparency without jargon**: show limits, fees, and conditions up front in plain language, at the moment they matter.
- **Progressive disclosure**: keep advanced details available but tucked away by default.
- **Hide the rails**: payments ride on Lightning, Spark, and conversions under the hood. Users think in terms of people, dollars, and bitcoin, never in terms of plumbing.

## Guidelines

- **[Login & backup]**
- **[Displaying payments]**
- **[Receiving payments]**
- **[Sending payments]**

[Receiving payments]: uxguide_receive.md
[Sending payments]: uxguide_send.md
[Displaying payments]: uxguide_display.md
[Login & backup]: uxguide_login.md
