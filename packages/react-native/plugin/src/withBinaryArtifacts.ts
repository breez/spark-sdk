import type { ConfigPlugin } from '@expo/config-plugins';
import * as path from 'path';
import * as fs from 'fs';
import { execSync } from 'child_process';

/**
 * Downloads prebuilt binary artifacts for Android and iOS
 * This runs during expo prebuild to ensure binaries are available
 */
export const withBinaryArtifacts: ConfigPlugin = (config) => {
  return {
    ...config,
    async prebuildAsync(config: any) {
      try {
        await downloadBinaryArtifacts();
      } catch (error) {
        console.warn('Failed to download Breez SDK binary artifacts:', error);
        console.warn(
          'You may need to run the postinstall script manually or check your network connection.'
        );
      }
      return config;
    },
  } as any;
};

async function downloadBinaryArtifacts(): Promise<void> {
  const packageRoot = findPackageRoot();
  if (!packageRoot) {
    throw new Error(
      'Could not find @breeztech/breez-sdk-spark-react-native package'
    );
  }

  // Check if artifacts already exist
  const androidLibsPath = path.join(packageRoot, 'android/src/main/jniLibs');
  const iosFrameworkPath = path.join(
    packageRoot,
    'build/RnBreezSdkSpark.xcframework'
  );

  if (fs.existsSync(androidLibsPath) && fs.existsSync(iosFrameworkPath)) {
    return;
  }

  const packageJsonPath = path.join(packageRoot, 'package.json');
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf-8'));
  const version = packageJson.version;
  const androidChecksum = packageJson.checksums?.android;
  const iosChecksum = packageJson.checksums?.ios;

  if (!androidChecksum || !iosChecksum) {
    throw new Error('Binary checksums not found in package.json');
  }

  const repo = 'https://github.com/breez/breez-sdk-spark-react-native';
  const androidUrl = `${repo}/releases/download/${version}/android-artifacts.zip`;
  const iosUrl = `${repo}/releases/download/${version}/ios-artifacts.zip`;

  // Download and verify Android artifacts
  try {
    execSync(`curl -L "${androidUrl}" --output android-artifacts.zip`, {
      cwd: packageRoot,
      stdio: 'inherit',
    });

    const actualAndroidChecksum = execSync(
      'shasum -a 256 android-artifacts.zip | cut -d" " -f1',
      { cwd: packageRoot, encoding: 'utf-8' }
    ).trim();

    if (actualAndroidChecksum !== androidChecksum) {
      throw new Error(
        `Android artifacts checksum mismatch. Expected: ${androidChecksum}, Got: ${actualAndroidChecksum}`
      );
    }

    execSync('unzip -o android-artifacts.zip && rm -rf android-artifacts.zip', {
      cwd: packageRoot,
      stdio: 'inherit',
    });
  } catch (error) {
    execSync('rm -f android-artifacts.zip', { cwd: packageRoot });
    console.error('Failed to download or verify Android artifacts');
    throw error;
  }

  // Download and verify iOS artifacts
  try {
    execSync(`curl -L "${iosUrl}" --output ios-artifacts.zip`, {
      cwd: packageRoot,
      stdio: 'inherit',
    });

    const actualIosChecksum = execSync(
      'shasum -a 256 ios-artifacts.zip | cut -d" " -f1',
      { cwd: packageRoot, encoding: 'utf-8' }
    ).trim();

    if (actualIosChecksum !== iosChecksum) {
      throw new Error(
        `iOS artifacts checksum mismatch. Expected: ${iosChecksum}, Got: ${actualIosChecksum}`
      );
    }

    execSync('unzip -o ios-artifacts.zip && rm -rf ios-artifacts.zip', {
      cwd: packageRoot,
      stdio: 'inherit',
    });
  } catch (error) {
    execSync('rm -f ios-artifacts.zip', { cwd: packageRoot });
    console.error('Failed to download or verify iOS artifacts');
    throw error;
  }
}

function findPackageRoot(): string | null {
  let currentDir = __dirname;

  // Walk up the directory tree to find the package root
  while (currentDir !== path.dirname(currentDir)) {
    const packageJsonPath = path.join(currentDir, 'package.json');

    if (fs.existsSync(packageJsonPath)) {
      const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf-8'));
      if (packageJson.name === '@breeztech/breez-sdk-spark-react-native') {
        return currentDir;
      }
    }

    currentDir = path.dirname(currentDir);
  }

  return null;
}
