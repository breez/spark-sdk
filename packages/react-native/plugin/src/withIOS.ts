import { type ConfigPlugin, withEntitlementsPlist } from '@expo/config-plugins';

type WithIOSOptions = {
  enableSeedlessRestore: boolean;
};

/**
 * Configure iOS build settings for Breez SDK
 * The podspec already defines the minimum iOS version via min_ios_version_supported
 */
export const withBreezSdkIOS: ConfigPlugin<WithIOSOptions> = (
  config,
  { enableSeedlessRestore }
) => {
  if (!enableSeedlessRestore) {
    return config;
  }

  return withEntitlementsPlist(config, (config) => {
    const domain = 'webcredentials:keys.breez.technology';
    const domains: string[] =
      (config.modResults['com.apple.developer.associated-domains'] as string[] | undefined) ?? [];

    if (!domains.includes(domain)) {
      domains.push(domain);
    }

    config.modResults['com.apple.developer.associated-domains'] = domains;
    return config;
  });
};
