import { type ConfigPlugin, withGradleProperties } from '@expo/config-plugins';

/**
 * Add required configurations to gradle.properties
 */
const withGradlePropertiesConfig: ConfigPlugin = (config) => {
  return withGradleProperties(config, (config) => {
    config.modResults = config.modResults.filter(
      (item) => item.type !== 'property' || item.key !== 'android.useAndroidX'
    );

    config.modResults.push({
      type: 'property',
      key: 'android.useAndroidX',
      value: 'true',
    });

    return config;
  });
};

/**
 * Configure Android build settings for Breez SDK
 */
export const withBreezSdkAndroid: ConfigPlugin = (config) => {
  config = withGradlePropertiesConfig(config);

  return config;
};
