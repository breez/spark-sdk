'use strict'

const fs = require('fs')
const path = require('path')
const bip39 = require('bip39')

const PHRASE_FILE_NAME = 'phrase'
const HISTORY_FILE_NAME = 'history.txt'

class CliPersistence {
  /**
   * @param {string} dataDir - Path to the data directory
   */
  constructor(dataDir) {
    this.dataDir = dataDir
  }

  /**
   * Reads an existing mnemonic from the data directory, or generates a new
   * 12-word BIP39 mnemonic and saves it.
   *
   * @returns {string} The mnemonic phrase
   */
  getOrCreateMnemonic() {
    const filename = path.join(this.dataDir, PHRASE_FILE_NAME)

    try {
      const phrase = fs.readFileSync(filename, 'utf-8').trim()
      if (phrase) {
        return phrase
      }
    } catch (err) {
      if (err.code !== 'ENOENT') {
        throw new Error(`Can't read from file ${filename}: ${err.message}`)
      }
    }

    // Generate a new 12-word mnemonic (128 bits of entropy)
    const mnemonic = bip39.generateMnemonic(128)
    fs.mkdirSync(path.dirname(filename), { recursive: true })
    fs.writeFileSync(filename, mnemonic, { mode: 0o600 })
    return mnemonic
  }

  /**
   * Returns the path to the REPL history file.
   *
   * @returns {string} Path to the history file
   */
  historyFile() {
    return path.join(this.dataDir, HISTORY_FILE_NAME)
  }
}

module.exports = { CliPersistence }
