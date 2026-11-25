import 'dart:convert';
import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:crypto/crypto.dart';
import 'package:hooks/hooks.dart';

import 'assets.dart';

/// Bundles the SDK's native library and its callback relay library.
///
/// Inside the spark-sdk repository both are built from source with cargo.
/// Everywhere else they are downloaded from the release listed in
/// `prebuilt.json` and checked against its sha256 values. Releases cover
/// Android, iOS and macOS. Other platforms get no library, so an app still
/// builds for them, and SDK calls fail there at runtime.
///
/// Optional user-defines, under `hooks: user_defines: breez_sdk_spark_flutter:`
/// in the app's pubspec:
/// - `build_from_source`: `true` or `false`, overriding the repository check.
/// - `cargo_profile`: the cargo profile for source builds, `release` by default.
/// - `prebuilt_dir`: a local directory holding the files `prebuilt.json` lists.
/// - `prebuilt_base_url`: a mirror to download those files from instead.
void main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) return;
    final code = input.config.code;
    final repoRoot = input.packageRoot.resolve('../../');
    final buildFromSource = switch (input.userDefines['build_from_source']) {
      final bool value => value,
      null => File.fromUri(repoRoot.resolve('crates/breez-sdk/bindings/Cargo.toml')).existsSync(),
      final other => throw FormatException('build_from_source must be a boolean, got $other'),
    };
    if (!buildFromSource && !_releasedOSes.contains(code.targetOS)) {
      stderr.writeln(
        'breez_sdk_spark_flutter: no ${code.targetOS} library is released, so none is bundled '
        'and SDK calls fail on ${code.targetOS}.',
      );
      return;
    }

    final triple = _rustTriple(code);
    if (triple == null) {
      throw UnsupportedError(
        'breez_sdk_spark_flutter: ${code.targetOS} ${code.targetArchitecture} is not supported',
      );
    }

    final libraries = buildFromSource
        ? await _buildFromSource(input, output, code, triple, repoRoot)
        : await _fetchPrebuilt(input, output, code, triple);
    for (final MapEntry(key: asset, value: file) in libraries.entries) {
      output.assets.code.add(
        CodeAsset(package: input.packageName, name: asset, linkMode: DynamicLoadingBundled(), file: file),
      );
    }
  });
}

final _releasedOSes = {OS.android, OS.iOS, OS.macOS};

String? _rustTriple(CodeConfig code) {
  final os = code.targetOS;
  final arch = code.targetArchitecture;
  if (os == OS.android) {
    if (arch == Architecture.arm) return 'armv7-linux-androideabi';
    if (arch == Architecture.arm64) return 'aarch64-linux-android';
    if (arch == Architecture.ia32) return 'i686-linux-android';
    if (arch == Architecture.x64) return 'x86_64-linux-android';
  } else if (os == OS.iOS) {
    final simulator = code.iOS.targetSdk == IOSSdk.iPhoneSimulator;
    if (arch == Architecture.arm64) return simulator ? 'aarch64-apple-ios-sim' : 'aarch64-apple-ios';
    if (arch == Architecture.x64 && simulator) return 'x86_64-apple-ios';
  } else if (os == OS.macOS) {
    if (arch == Architecture.arm64) return 'aarch64-apple-darwin';
    if (arch == Architecture.x64) return 'x86_64-apple-darwin';
  } else if (os == OS.linux) {
    if (arch == Architecture.arm64) return 'aarch64-unknown-linux-gnu';
    if (arch == Architecture.x64) return 'x86_64-unknown-linux-gnu';
  } else if (os == OS.windows) {
    if (arch == Architecture.x64) return 'x86_64-pc-windows-msvc';
  }
  return null;
}

String _libraryFileName(OS os, String name) {
  if (os == OS.windows) return '$name.dll';
  if (os == OS.iOS || os == OS.macOS) return 'lib$name.dylib';
  return 'lib$name.so';
}

// Source builds

const _mobileFeatures = 'db-storage-api,secp-lowmemory,release-max-level-debug';

Future<Map<String, Uri>> _buildFromSource(
  HookInput input,
  HookOutputBuilder output,
  CodeConfig code,
  String triple,
  Uri repoRoot,
) async {
  final profile = switch (input.userDefines['cargo_profile']) {
    final String value => value,
    null => 'release',
    final other => throw FormatException('cargo_profile must be a string, got $other'),
  };
  final profileDir = switch (profile) {
    'release' => 'release',
    'dev' => 'debug',
    _ => profile,
  };
  // A separate target directory, so hook builds with the mobile feature set
  // don't invalidate other builds in the workspace's target directory.
  final targetDir = repoRoot.resolve('target/flutter-hook/');
  final relayManifest = input.packageRoot.resolve('native/dispatch/Cargo.toml');

  final crates = <String, List<String>>{
    bindingsAsset: ['-p', 'breez-sdk-spark-bindings', '--no-default-features', '--features', _mobileFeatures],
    if (File.fromUri(relayManifest).existsSync()) relayAsset: ['--manifest-path', relayManifest.toFilePath()],
  };
  final libraries = <String, Uri>{};
  for (final MapEntry(key: asset, value: crateArgs) in crates.entries) {
    await _cargoBuild(code, triple, profile, targetDir, repoRoot, crateArgs);
    final outDir = targetDir.resolve('$triple/$profileDir/');
    final name = libraryNames[asset]!;
    final library = outDir.resolve(_libraryFileName(code.targetOS, name));
    if (!File.fromUri(library).existsSync()) {
      throw StateError('cargo did not produce ${library.toFilePath()}');
    }
    libraries[asset] = library;
    output.dependencies.addAll(_sourceDependencies(outDir.resolve('lib$name.d'), repoRoot));
  }
  output.dependencies
    ..add(repoRoot.resolve('Cargo.lock'))
    ..add(repoRoot.resolve('.cargo/config.toml'));
  return libraries;
}

Future<void> _cargoBuild(
  CodeConfig code,
  String triple,
  String profile,
  Uri targetDir,
  Uri repoRoot,
  List<String> crateArgs,
) async {
  final environment = <String, String>{};
  final List<String> command;
  final buildArgs = [
    if (profile == 'release') '--release' else ...['--profile', profile],
    '--target-dir',
    targetDir.toFilePath(),
    ...crateArgs,
  ];
  if (code.targetOS == OS.android) {
    final ndk = _ndkRoot(code);
    if (ndk != null) environment['ANDROID_NDK_HOME'] = ndk;
    final abi = switch (triple) {
      'armv7-linux-androideabi' => 'armeabi-v7a',
      'aarch64-linux-android' => 'arm64-v8a',
      'i686-linux-android' => 'x86',
      _ => 'x86_64',
    };
    command = ['ndk', '-t', abi, '-P', '${code.android.targetNdkApi}', 'build', ...buildArgs];
  } else {
    if (code.targetOS == OS.iOS) environment['IPHONEOS_DEPLOYMENT_TARGET'] = '15.0';
    if (code.targetOS == OS.macOS) environment['MACOSX_DEPLOYMENT_TARGET'] = '10.15';
    command = ['build', '--target', triple, ...buildArgs];
  }

  // Run from the repository root so .cargo/config.toml applies.
  final process = await Process.start(
    _cargo(),
    command,
    workingDirectory: repoRoot.toFilePath(),
    environment: environment,
  );
  final stderrLines = <String>[];
  process.stdout.transform(utf8.decoder).listen(stdout.write);
  process.stderr.transform(utf8.decoder).transform(const LineSplitter()).listen((line) {
    stderr.writeln(line);
    stderrLines.add(line);
  });
  final exitCode = await process.exitCode;
  if (exitCode != 0) {
    final tail = stderrLines.skip(stderrLines.length > 40 ? stderrLines.length - 40 : 0).join('\n');
    throw ProcessException(_cargo(), command, 'cargo failed for $triple:\n$tail', exitCode);
  }
}

String _cargo() {
  final home = Platform.environment['HOME'] ?? Platform.environment['USERPROFILE'];
  final installed = home == null ? null : File('$home/.cargo/bin/cargo${Platform.isWindows ? '.exe' : ''}');
  return installed != null && installed.existsSync() ? installed.path : 'cargo';
}

/// The NDK root, derived from the clang Flutter passes for the target:
/// `<ndk>/toolchains/llvm/prebuilt/<host>/bin/clang`.
String? _ndkRoot(CodeConfig code) {
  final compiler = code.cCompiler?.compiler;
  if (compiler == null) return null;
  return compiler.resolve('../../../../../').toFilePath();
}

/// The repository sources cargo read for a build, from its dep-info file.
Iterable<Uri> _sourceDependencies(Uri depInfo, Uri repoRoot) {
  final file = File.fromUri(depInfo);
  if (!file.existsSync()) return const [];
  final root = repoRoot.toFilePath();
  final targets = file.readAsStringSync().split('\n').first;
  final colon = targets.indexOf(': ');
  if (colon < 0) return const [];
  return targets
      .substring(colon + 2)
      .split(RegExp(r'(?<!\\) '))
      .map((path) => path.replaceAll(r'\ ', ' ').trim())
      .where((path) => path.startsWith(root) && !path.contains('/target/'))
      // Build scripts list whole directories, such as the proto sources.
      .map((path) => FileSystemEntity.isDirectorySync(path) ? Uri.directory(path) : Uri.file(path));
}

// Prebuilt libraries

Future<Map<String, Uri>> _fetchPrebuilt(
  HookInput input,
  HookOutputBuilder output,
  CodeConfig code,
  String triple,
) async {
  final manifestUri = input.packageRoot.resolve('hook/prebuilt.json');
  output.dependencies.add(manifestUri);
  final manifest = jsonDecode(File.fromUri(manifestUri).readAsStringSync()) as Map<String, Object?>;
  final targets = manifest['targets'] as Map<String, Object?>;
  final entry = targets[triple] as Map<String, Object?>?;
  if (entry == null) {
    throw UnsupportedError(
      'breez_sdk_spark_flutter: no prebuilt library for $triple. '
      'Set the build_from_source user-define to build it with cargo.',
    );
  }

  // A directory URI, so file names resolve inside it rather than next to it.
  final prebuiltPath = input.userDefines.path('prebuilt_dir');
  final prebuiltDir = prebuiltPath == null ? null : Uri.directory(prebuiltPath.toFilePath());
  final baseUrl = switch (input.userDefines['prebuilt_base_url']) {
    final String value => value.endsWith('/') ? value : '$value/',
    null => manifest['base_url'] as String?,
    final other => throw FormatException('prebuilt_base_url must be a string, got $other'),
  };

  final libraries = <String, Uri>{};
  for (final asset in [bindingsAsset, relayAsset]) {
    final file = entry[asset] as Map<String, Object?>?;
    if (file == null) continue;
    libraries[asset] = await _fetch(
      input: input,
      output: output,
      fileName: file['file'] as String,
      sha256: file['sha256'] as String,
      libraryFileName: _libraryFileName(code.targetOS, libraryNames[asset]!),
      prebuiltDir: prebuiltDir,
      baseUrl: baseUrl,
    );
  }
  if (!libraries.containsKey(bindingsAsset)) {
    throw StateError('prebuilt.json lists no $bindingsAsset library for $triple');
  }
  return libraries;
}

/// Returns the unpacked library for a gzipped release file, downloading it or
/// copying it from [prebuiltDir] into the shared cache when needed. The
/// gzipped file is checked against [sha256] on every build.
Future<Uri> _fetch({
  required HookInput input,
  required HookOutputBuilder output,
  required String fileName,
  required String sha256,
  required String libraryFileName,
  required Uri? prebuiltDir,
  required String? baseUrl,
}) async {
  final dir = Directory.fromUri(input.outputDirectoryShared.resolve('$sha256/'));
  final archive = File('${dir.path}$fileName');
  final library = File('${dir.path}$libraryFileName');

  if (!archive.existsSync() || _sha256(archive) != sha256) {
    dir.createSync(recursive: true);
    final partial = File('${archive.path}.partial');
    if (prebuiltDir != null) {
      final source = File.fromUri(prebuiltDir.resolve(fileName));
      output.dependencies.add(source.uri);
      source.copySync(partial.path);
    } else if (baseUrl != null) {
      await _download(Uri.parse(baseUrl).resolve(fileName), partial);
    } else {
      throw StateError('prebuilt.json has no base_url, and no prebuilt_dir user-define is set');
    }
    final actual = _sha256(partial);
    if (actual != sha256) {
      partial.deleteSync();
      throw StateError('$fileName has sha256 $actual, expected $sha256');
    }
    partial.renameSync(archive.path);
    if (library.existsSync()) library.deleteSync();
  }

  if (!library.existsSync()) {
    final partial = File('${library.path}.partial');
    partial.writeAsBytesSync(gzip.decode(archive.readAsBytesSync()));
    partial.renameSync(library.path);
  }
  return library.uri;
}

/// Downloads [url] to [destination]. A server that stops responding for a
/// minute fails the build instead of hanging it.
Future<void> _download(Uri url, File destination) async {
  const timeout = Duration(seconds: 60);
  final client = HttpClient()..connectionTimeout = timeout;
  try {
    final request = await client.getUrl(url);
    final response = await request.close().timeout(timeout);
    if (response.statusCode != HttpStatus.ok) {
      throw HttpException('status ${response.statusCode}', uri: url);
    }
    await response.timeout(timeout).pipe(destination.openWrite());
  } on Exception catch (e) {
    throw StateError(
      'breez_sdk_spark_flutter: downloading $url failed: $e\n'
      'To build without downloading, put the files hook/prebuilt.json lists in a directory '
      'and set the prebuilt_dir user-define to it.',
    );
  } finally {
    client.close(force: true);
  }
}

String _sha256(File file) => sha256.convert(file.readAsBytesSync()).toString();
