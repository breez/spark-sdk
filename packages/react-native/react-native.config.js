/**
 * @type {import('@react-native-community/cli-types').UserDependencyConfig}
 */
module.exports = {
  dependency: {
    platforms: {
      android: {
        cmakeListsPath: 'android/app/build/generated/source/codegen/jni/CMakeLists.txt',
      },
    },
  },
};