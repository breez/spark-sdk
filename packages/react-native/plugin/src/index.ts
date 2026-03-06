import {
  type ConfigPlugin,
  withPlugins,
  createRunOncePlugin,
} from '@expo/config-plugins';
import { withBinaryArtifacts } from './withBinaryArtifacts';
import { withBreezSdkAndroid } from './withAndroid';
import { withBreezSdkIOS } from './withIOS';
import { sdkPackage } from './utils';

export type BreezSdkPluginOptions = {
  /**
   * Skip downloading binary artifacts (default: false)
   * Set to true if you want to handle binary downloads manually
   */
  skipBinaryDownload?: boolean;
  /**
   * Add webcredentials:keys.breez.technology to the iOS Associated Domains
   * entitlement, required for passkey-based seed derivation (default: false)
   */
  enablePasskey?: boolean;
};

const withBreezSdk: ConfigPlugin<BreezSdkPluginOptions | void> = (
  config,
  options
) => {
  const { skipBinaryDownload = false, enablePasskey = false } =
    options || {};

  return withPlugins(config, [
    // Download binary artifacts first
    ...(skipBinaryDownload ? [] : [withBinaryArtifacts]),
    // Configure Android
    withBreezSdkAndroid,
    // Configure iOS
    [withBreezSdkIOS, { enablePasskey }] as const,
  ]);
};

export default createRunOncePlugin(
  withBreezSdk,
  sdkPackage.name,
  sdkPackage.version
);
