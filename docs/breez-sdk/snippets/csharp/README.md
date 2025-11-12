# Breez SDK C# Snippets

This project contains C# code snippets for the Breez SDK documentation.

## Prerequisites

- .NET 8.0 SDK or later
- Breez SDK C# bindings (Breez.Sdk.Spark NuGet package)

## Setup

1. Restore dependencies:
```bash
dotnet restore
```

2. Build the project:
```bash
dotnet build
```

## Structure

Each snippet file corresponds to a documentation page and contains code examples wrapped in anchor comments:

```csharp
// ANCHOR: snippet-name
// Your code here
// ANCHOR_END: snippet-name
```

These anchors are used by the documentation preprocessor to extract and display the snippets in the documentation.
