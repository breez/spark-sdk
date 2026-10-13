'use strict'

// This project installs neither `pg` nor `mysql2`. The package loads without
// a warning about them, and tells an app that selects PostgreSQL or MySQL
// storage which driver to install. No network.

const assert = require('node:assert/strict')
const test = require('node:test')

// The package prints its warnings on the first require in a process.
const warnings = []
const consoleWarn = console.warn
console.warn = (...args) => warnings.push(args.join(' '))
const {
  SdkBuilder,
  defaultConfig,
  defaultMysqlStorageConfig,
  defaultPostgresStorageConfig,
  mysqlStorage,
  postgresStorage,
} = require('@breeztech/breez-sdk-spark/nodejs')
console.warn = consoleWarn

function buildWith(storage) {
  const mnemonic = require('bip39').generateMnemonic()
  const builder = SdkBuilder.new(defaultConfig('regtest'), { type: 'mnemonic', mnemonic })
  return builder.withStorageBackend(storage).build()
}

test('loading the package prints no database driver warning', () => {
  assert.deepEqual(
    warnings.filter((warning) => /PostgreSQL|MySQL/.test(warning)),
    []
  )
})

test('selecting PostgreSQL storage names the missing driver', async () => {
  const config = defaultPostgresStorageConfig('postgres://user:password@localhost:5432/spark')
  await assert.rejects(buildWith(postgresStorage(config)), /pg not found\. Please install it/)
})

test('selecting MySQL storage names the missing driver', async () => {
  const config = defaultMysqlStorageConfig('mysql://user:password@localhost:3306/spark')
  await assert.rejects(buildWith(mysqlStorage(config)), /mysql2 not found\. Please install it/)
})
