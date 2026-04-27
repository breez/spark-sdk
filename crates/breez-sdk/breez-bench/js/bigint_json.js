'use strict'

// Allow JSON.stringify to handle BigInt values produced by the SDK.
if (typeof BigInt !== 'undefined' && !BigInt.prototype.toJSON) {
  // eslint-disable-next-line no-extend-native
  BigInt.prototype.toJSON = function () {
    return this.toString()
  }
}
