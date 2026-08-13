// Root shim so `@breeztech/breez-sdk-spark-react-native/passkey-prf-provider`
// resolves on React Native versions whose Metro ignores package `exports`
// (RN < 0.79, which the package still supports). Metro versions that honour
// `exports` map the subpath to lib/ directly and never read this file.
module.exports = require('./lib/commonjs/passkey-prf-provider.js');
