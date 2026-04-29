#!/usr/bin/env node
/**
 * Apply hand-edits to uniffi-bindgen-react-native-generated files after
 * running `yarn ubrn:android`, `yarn ubrn:ios`, or `yarn ubrn:checkout`.
 *
 * The code generator is unaware of the built-in passkey PRF provider, so
 * each regeneration would otherwise silently drop the few lines required
 * to:
 *
 *   1. android/build.gradle
 *        - add androidx.credentials / kotlinx-coroutines dependencies
 *          (needed by the hand-written BreezSdkSparkPasskeyModule.kt and
 *          CredentialManagerPrfCore.kt)
 *        - add `src/main/kotlin` to the Android gradle sourceSets so
 *          those hand-written files (which live under
 *          `android/src/main/kotlin/...` to survive `yarn ubrn:clean`)
 *          are actually compiled
 *
 *   2. android/src/main/java/.../BreezSdkSparkReactNativePackage.kt
 *        - register BreezSdkSparkPasskeyModule alongside the generated
 *          UniFFI TurboModule so React Native can find it at runtime
 *
 * The PasskeyPrfProvider class is exposed via a subpath export
 * (`@breeztech/breez-sdk-spark-react-native/passkey-prf-provider`) declared
 * in package.json `exports`, so no edit to the generated `src/index.tsx`
 * is required.
 *
 * Each patch is idempotent (runs as a no-op if already applied) and
 * errors loudly if its anchor cannot be found. If uniffi-bindgen-react-native
 * changes its output format, this script fails fast with an actionable
 * message instead of silently producing a broken package.
 *
 * Invoked automatically by the `ubrn:*` npm scripts; can also be run
 * manually via `yarn post-ubrn`.
 */

'use strict';

const fs = require('fs');
const path = require('path');

const args = process.argv.slice(2);
const checkMode = args.includes('--check') || args.includes('--dry-run');

const repoRoot = path.resolve(__dirname, '..');
const drift = [];
const errors = [];

function patchFile(relPath, label, patcher) {
  const filePath = path.join(repoRoot, relPath);
  if (!fs.existsSync(filePath)) {
    console.log(`[post-ubrn] ${label}: skipping, file not yet generated (${relPath})`);
    return;
  }
  let before, after;
  try {
    before = fs.readFileSync(filePath, 'utf8');
    after = patcher(before, label, relPath);
  } catch (err) {
    errors.push({ label, relPath, message: err.message });
    return;
  }
  if (before === after) {
    console.log(`[post-ubrn] ${label}: already patched (${relPath})`);
    return;
  }
  if (checkMode) {
    drift.push({ label, relPath });
    console.log(`[post-ubrn] ${label}: WOULD PATCH (${relPath})`);
    return;
  }
  fs.writeFileSync(filePath, after);
  console.log(`[post-ubrn] ${label}: patched (${relPath})`);
}

function requireAnchor(content, anchor, label, relPath) {
  if (!content.includes(anchor)) {
    throw new Error(
      `anchor not found. The uniffi-bindgen-react-native output format may ` +
        `have changed (check the installed version against the one this ` +
        `script was written for). See CLAUDE.md "Generated Files Policy" ` +
        `for how to update the patches. Expected anchor text:\n${anchor}`
    );
  }
}

// ---------------------------------------------------------------------------
// 1. android/build.gradle
// ---------------------------------------------------------------------------

patchFile(
  'android/build.gradle',
  'androidx.credentials dependencies',
  (content, label, relPath) => {
    if (content.includes('androidx.credentials:credentials:1.3.0')) {
      return content;
    }
    const anchor = '  implementation "org.jetbrains.kotlin:kotlin-stdlib:$kotlin_version"';
    requireAnchor(content, anchor, label, relPath);
    const injected = [
      anchor,
      '  implementation "androidx.credentials:credentials:1.3.0"',
      '  implementation "androidx.credentials:credentials-play-services-auth:1.3.0"',
      '  implementation "org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.0"',
    ].join('\n');
    return content.replace(anchor, injected);
  }
);

patchFile(
  'android/build.gradle',
  'src/main/kotlin sourceSet',
  (content, label, relPath) => {
    if (content.includes("main.kotlin.srcDirs += 'src/main/kotlin'")) {
      return content;
    }
    // The generated build.gradle already has a `sourceSets { main { ... } }`
    // block (for the new-architecture jni dirs). Extend it by adding the
    // hand-written kotlin source dir right after the opening `main {`.
    const anchor = `  sourceSets {
    main {
      if (isNewArchitectureEnabled()) {`;
    requireAnchor(content, anchor, label, relPath);
    return content.replace(
      anchor,
      `  sourceSets {
    main {
      main.kotlin.srcDirs += 'src/main/kotlin'
      if (isNewArchitectureEnabled()) {`
    );
  }
);

// ---------------------------------------------------------------------------
// 2. android/src/main/java/.../BreezSdkSparkReactNativePackage.kt
// ---------------------------------------------------------------------------

const PACKAGE_KT_REL =
  'android/src/main/java/com/breeztech/breezsdkspark/BreezSdkSparkReactNativePackage.kt';

patchFile(
  PACKAGE_KT_REL,
  'BreezSdkSparkPasskeyModule getModule() registration',
  (content, label, relPath) => {
    if (content.includes('BreezSdkSparkPasskeyModule.NAME ->')) {
      return content;
    }
    const anchor = `  override fun getModule(name: String, reactContext: ReactApplicationContext): NativeModule? {
    return if (name == BreezSdkSparkReactNativeModule.NAME) {
      BreezSdkSparkReactNativeModule(reactContext)
    } else {
      null
    }
  }`;
    const replacement = `  override fun getModule(name: String, reactContext: ReactApplicationContext): NativeModule? {
    return when (name) {
      BreezSdkSparkReactNativeModule.NAME -> BreezSdkSparkReactNativeModule(reactContext)
      BreezSdkSparkPasskeyModule.NAME -> BreezSdkSparkPasskeyModule(reactContext)
      else -> null
    }
  }`;
    requireAnchor(content, anchor, label, relPath);
    return content.replace(anchor, replacement);
  }
);

patchFile(
  PACKAGE_KT_REL,
  'BreezSdkSparkPasskeyModule ReactModuleInfo registration',
  (content, label, relPath) => {
    if (content.includes('moduleInfos[BreezSdkSparkPasskeyModule.NAME]')) {
      return content;
    }
    const anchor = `        true // isTurboModule
      )
      moduleInfos
    }
  }
}`;
    const replacement = `        true // isTurboModule
      )
      moduleInfos[BreezSdkSparkPasskeyModule.NAME] = ReactModuleInfo(
        BreezSdkSparkPasskeyModule.NAME,
        BreezSdkSparkPasskeyModule.NAME,
        false,  // canOverrideExistingModule
        false,  // needsEagerInit
        false,  // isCxxModule
        false // isTurboModule (standard native module)
      )
      moduleInfos
    }
  }
}`;
    requireAnchor(content, anchor, label, relPath);
    return content.replace(anchor, replacement);
  }
);

if (errors.length > 0) {
  console.error('');
  console.error(`[post-ubrn] ${errors.length} patch(es) failed:`);
  for (const { label, relPath, message } of errors) {
    console.error(`  - ${label} (${relPath})`);
    console.error(`    ${message.split('\n').join('\n    ')}`);
  }
  process.exit(2);
}

if (checkMode && drift.length > 0) {
  console.error('');
  console.error(
    `[post-ubrn] ${drift.length} generated file(s) are out of sync with the ` +
      `committed hand-edits:`
  );
  for (const { label, relPath } of drift) {
    console.error(`  - ${label} (${relPath})`);
  }
  console.error('');
  console.error(
    'Run `yarn post-ubrn` inside packages/react-native and commit the diff, ' +
      'or revert whatever change dropped the patches. See CLAUDE.md ' +
      '"Generated Files Policy" for context.'
  );
  process.exit(1);
}

console.log('[post-ubrn] done.');
