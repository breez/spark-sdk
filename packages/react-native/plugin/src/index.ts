import {
  type ConfigPlugin,
  withPlugins,
  createRunOncePlugin,
} from '@expo/config-plugins';
import { withBreezSdkAndroid } from './withAndroid';
import { withBreezSdkIOS } from './withIOS';
import { sdkPackage } from './utils';

export type BreezSdkPluginOptions = {};

const withBreezSdk: ConfigPlugin<BreezSdkPluginOptions | void> = (
  config,
  options
) => {
  const {} = options || {};

  return withPlugins(config, [
    // Configure Android
    withBreezSdkAndroid,
    // Configure iOS
    withBreezSdkIOS,
  ]);
};

export default createRunOncePlugin(
  withBreezSdk,
  sdkPackage.name,
  sdkPackage.version
);
