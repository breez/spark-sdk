const path = require('path')
const { getDefaultConfig, mergeConfig } = require('@react-native/metro-config')

// The SDK package is a file: dependency (symlinked).
// Metro needs to know where to find it and its node_modules.
const sdkPath = path.resolve(__dirname, '../../../../../../../packages/react-native')

const config = {
  watchFolders: [sdkPath],
  resolver: {
    nodeModulesPaths: [
      path.resolve(__dirname, 'node_modules'),
      path.resolve(sdkPath, 'node_modules'),
    ],
  },
}

module.exports = mergeConfig(getDefaultConfig(__dirname), config)
