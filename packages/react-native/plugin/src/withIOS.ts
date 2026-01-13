import { type ConfigPlugin } from '@expo/config-plugins';

/**
 * Configure iOS build settings for Breez SDK
 * The podspec already defines the minimum iOS version via min_ios_version_supported
 */
export const withBreezSdkIOS: ConfigPlugin = (config) => {
  // Currently no additional iOS configuration needed
  // The podspec handles minimum version and framework linking
  return config;
};
