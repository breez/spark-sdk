'use strict'

// Pure tests for the tolerant JSON evaluator; no network. Mirrors the unit
// tests in crates/breez-sdk/cli/tests/harness/assert.rs.

const assert = require('node:assert/strict')
const test = require('node:test')

const { checkMatcher, extractJsonDocs, interpolate, lookupPath } = require('./scenario')

test('extracts pretty and inline docs and skips noise', () => {
  const chunk = 'Breez SDK: noise\n{\n  "a": 1\n}\nError: nope\n{"b": 2}\nEvent: {"c": 3}\n'
  assert.deepEqual(extractJsonDocs(chunk), [{ a: 1 }, { b: 2 }])
})

test('path matches across casings', () => {
  const rust = { payment_request: { amount_msat: 5 } }
  const wasm = { paymentRequest: { amountMsat: 5 } }
  for (const doc of [rust, wasm]) {
    assert.equal(lookupPath(doc, 'payment_request.amount_msat'), 5)
  }
})

test('path bridges enum tags', () => {
  const rust = { Bolt11Invoice: { amount_msat: 7 } }
  const wasm = { type: 'bolt11Invoice', amountMsat: 7 }
  for (const doc of [rust, wasm]) {
    assert.equal(lookupPath(doc, 'bolt11_invoice.amount_msat'), 7)
  }
})

test('empty path addresses the document', () => {
  assert.deepEqual(lookupPath({}, ''), {})
  checkMatcher({}, lookupPath({}, ''))
})

test('path indexes arrays', () => {
  const doc = { payments: [{ id: 'x' }, { id: 'y' }] }
  assert.equal(lookupPath(doc, 'payments.1.id'), 'y')
  assert.equal(lookupPath(doc, 'payments.2.id'), undefined)
})

test('equality tolerates case and bigint strings', () => {
  checkMatcher('completed', 'Completed')
  checkMatcher('1000', 1000)
  checkMatcher(1000, '1000')
  assert.throws(() => checkMatcher('completed', 'failed'))
  assert.throws(() => checkMatcher('completed', undefined))
})

test('gte accepts numbers and numeric strings', () => {
  checkMatcher({ gte: 10 }, 11)
  checkMatcher({ gte: 10 }, '10')
  assert.throws(() => checkMatcher({ gte: 10 }, 9))
  assert.throws(() => checkMatcher({ gte: 10 }, 'abc'))
  assert.throws(() => checkMatcher({ gte: 10 }, undefined))
})

test('exists checks presence and null', () => {
  checkMatcher({ exists: true }, 'x')
  checkMatcher({ exists: false }, undefined)
  checkMatcher({ exists: false }, null)
  assert.throws(() => checkMatcher({ exists: true }, null))
  assert.throws(() => checkMatcher({ exists: true }, undefined))
})

test('interpolation substitutes known vars and rejects unknown', () => {
  assert.equal(interpolate('pay -r ${addr} -a 5', { addr: 'spark1x' }), 'pay -r spark1x -a 5')
  assert.throws(() => interpolate('${missing}', {}))
})
