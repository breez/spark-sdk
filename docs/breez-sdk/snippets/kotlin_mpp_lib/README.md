## Steps to use a local version of the SDK

```bash
cd ../../../../crates/breez-sdk/bindings
make package-kotlin-multiplatform
cd langs/kotlin-multiplatform
./gradlew publishToMavenLocal -PlibraryVersion=<local version>
```

Then set the version of the SDK in `gradle/libs.versions.toml` to the defined local version.

## Steps to compile the snippets locally

```bash
cd snippets/kotlin_mpp_lib/
./gradlew build
```

## Nix

Use the command `nix develop`
