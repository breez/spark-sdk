#!/usr/bin/env node
// Generates the React Native TS/C++ bindings + turbo-module template files
// without building the iOS xcframework.
//
// `ubrn build ios --and-generate` runs three things: a per-arch iOS Rust build,
// xcodebuild to package an xcframework, then bindgen. For per-PR doc-snippet
// checks we only need the bindgen output, so build a single host library
// instead and run ubrn's bindgen subcommands directly.
//
// uniffi reads its metadata from any built library regardless of architecture.
// The publishing flow (`yarn ubrn:ios` / `yarn ubrn:build`) is untouched.

import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import process from 'node:process';

const pkgDir = resolve(import.meta.dirname, '..');
const workspaceRoot = resolve(pkgDir, '../..');

function run(cmd, args, opts = {}) {
  console.log('>', cmd, args.join(' '));
  const result = spawnSync(cmd, args, { stdio: 'inherit', ...opts });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

// Build host library. `--no-default-features` mirrors ubrn.config.yaml's
// cargoExtras: postgres isn't needed for mobile bindings.
run('cargo', ['build', '-p', 'breez-sdk-bindings', '--no-default-features'], {
  cwd: workspaceRoot,
});

const libExt =
  process.platform === 'darwin'
    ? 'dylib'
    : process.platform === 'win32'
      ? 'dll'
      : 'so';
const libPrefix = process.platform === 'win32' ? '' : 'lib';
const hostLib = resolve(
  workspaceRoot,
  'target/debug',
  `${libPrefix}breez_sdk_spark_bindings.${libExt}`,
);

const ubrn = resolve(pkgDir, 'node_modules/.bin/ubrn');

// Generate TS/C++ bindings. Run from workspace root because the bindings
// subcommand calls `cargo metadata` against the cwd's Cargo.toml.
run(
  ubrn,
  [
    'generate',
    'jsi',
    'bindings',
    '--library',
    '--ts-dir',
    resolve(pkgDir, 'src/generated'),
    '--cpp-dir',
    resolve(pkgDir, 'cpp/generated'),
    hostLib,
  ],
  { cwd: workspaceRoot },
);

// Render turbo-module template files (podspec, CMakeLists, cpp-adapter, ...).
// Namespaces match the uniffi-exporting crates: `breez_sdk_spark` (the SDK
// re-export) and `breez_sdk_spark_bindings` (the wrapper crate's own
// scaffolding). Update these if either crate is renamed.
run(
  ubrn,
  [
    'generate',
    'jsi',
    'turbo-module',
    '--config',
    'ubrn.config.yaml',
    'breez_sdk_spark',
    'breez_sdk_spark_bindings',
  ],
  { cwd: pkgDir },
);
