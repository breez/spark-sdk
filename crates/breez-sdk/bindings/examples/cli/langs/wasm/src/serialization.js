'use strict'

// Enable BigInt JSON serialization
BigInt.prototype.toJSON = function () {
  return this.toString()
}

/**
 * Pretty-print a value as JSON to stdout.
 * Handles BigInt values via the toJSON prototype override above.
 *
 * @param {*} value - The value to print
 */
function printValue(value) {
  console.log(JSON.stringify(value, null, 2))
}

module.exports = { printValue }
