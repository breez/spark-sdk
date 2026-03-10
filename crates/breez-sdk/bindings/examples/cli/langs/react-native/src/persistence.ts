/**
 * CLI persistence for mnemonic and command history.
 *
 * Uses react-native-fs for file-based storage (mnemonic) and
 * AsyncStorage for command history.
 */

import RNFS from 'react-native-fs'
import AsyncStorage from '@react-native-async-storage/async-storage'

const PHRASE_FILE_NAME = 'phrase'
const HISTORY_STORAGE_KEY = '@breez_cli_history'
const MAX_HISTORY_ENTRIES = 500

export class CliPersistence {
  private dataDir: string

  constructor(dataDir: string) {
    this.dataDir = dataDir
  }

  /**
   * Get the storage directory path, creating it if necessary.
   */
  async ensureDataDir(): Promise<string> {
    const exists = await RNFS.exists(this.dataDir)
    if (!exists) {
      await RNFS.mkdir(this.dataDir)
    }
    return this.dataDir
  }

  /**
   * Read the mnemonic from the data directory, or generate a new 12-word
   * BIP39-compatible mnemonic and save it.
   *
   * Note: In a real app, you would use a proper BIP39 library. For this CLI
   * demo, we generate a random mnemonic placeholder. The actual mnemonic
   * generation should use a cryptographically secure method.
   */
  async getOrCreateMnemonic(): Promise<string> {
    await this.ensureDataDir()
    const filepath = `${this.dataDir}/${PHRASE_FILE_NAME}`

    const exists = await RNFS.exists(filepath)
    if (exists) {
      const mnemonic = await RNFS.readFile(filepath, 'utf8')
      return mnemonic.trim()
    }

    // Generate a simple random mnemonic using the BIP39 wordlist subset.
    // In production, use a proper BIP39 library.
    const mnemonic = generateSimpleMnemonic()
    await RNFS.writeFile(filepath, mnemonic, 'utf8')
    return mnemonic
  }

  /**
   * Add a command to the history.
   */
  async addToHistory(command: string): Promise<void> {
    try {
      const history = await this.getHistory()
      history.push(command)
      // Keep only the most recent entries
      const trimmed = history.slice(-MAX_HISTORY_ENTRIES)
      await AsyncStorage.setItem(HISTORY_STORAGE_KEY, JSON.stringify(trimmed))
    } catch {
      // Silently ignore history errors
    }
  }

  /**
   * Get the command history.
   */
  async getHistory(): Promise<string[]> {
    try {
      const raw = await AsyncStorage.getItem(HISTORY_STORAGE_KEY)
      if (raw) {
        return JSON.parse(raw) as string[]
      }
    } catch {
      // Ignore parse errors
    }
    return []
  }

  /**
   * Clear the command history.
   */
  async clearHistory(): Promise<void> {
    await AsyncStorage.removeItem(HISTORY_STORAGE_KEY)
  }
}

/**
 * Generate a simple 12-word mnemonic from a fixed BIP39-compatible word list.
 * This is a simplified implementation for CLI demo purposes.
 * In production, use a proper BIP39 library with cryptographically secure randomness.
 */
function generateSimpleMnemonic(): string {
  // Subset of BIP39 English wordlist for mnemonic generation
  const words = [
    'abandon', 'ability', 'able', 'about', 'above', 'absent', 'absorb', 'abstract',
    'absurd', 'abuse', 'access', 'accident', 'account', 'accuse', 'achieve', 'acid',
    'acoustic', 'acquire', 'across', 'act', 'action', 'actor', 'actress', 'actual',
    'adapt', 'add', 'addict', 'address', 'adjust', 'admit', 'adult', 'advance',
    'advice', 'aerobic', 'affair', 'afford', 'afraid', 'again', 'age', 'agent',
    'agree', 'ahead', 'aim', 'air', 'airport', 'aisle', 'alarm', 'album',
    'alcohol', 'alert', 'alien', 'all', 'alley', 'allow', 'almost', 'alone',
    'alpha', 'already', 'also', 'alter', 'always', 'amateur', 'amazing', 'among',
    'amount', 'amused', 'analyst', 'anchor', 'ancient', 'anger', 'angle', 'angry',
    'animal', 'ankle', 'announce', 'annual', 'another', 'answer', 'antenna', 'antique',
    'anxiety', 'any', 'apart', 'apology', 'appear', 'apple', 'approve', 'april',
    'arch', 'arctic', 'area', 'arena', 'argue', 'arm', 'armed', 'armor',
    'army', 'around', 'arrange', 'arrest', 'arrive', 'arrow', 'art', 'artefact',
    'artist', 'artwork', 'ask', 'aspect', 'assault', 'asset', 'assist', 'assume',
    'asthma', 'athlete', 'atom', 'attack', 'attend', 'attitude', 'attract', 'auction',
    'audit', 'august', 'aunt', 'author', 'auto', 'autumn', 'average', 'avocado',
    'avoid', 'awake', 'aware', 'awesome', 'awful', 'awkward', 'axis', 'baby',
    'bachelor', 'bacon', 'badge', 'bag', 'balance', 'balcony', 'ball', 'bamboo',
    'banana', 'banner', 'bar', 'barely', 'bargain', 'barrel', 'base', 'basic',
    'basket', 'battle', 'beach', 'bean', 'beauty', 'because', 'become', 'beef',
    'before', 'begin', 'behave', 'behind', 'believe', 'below', 'belt', 'bench',
    'benefit', 'best', 'betray', 'better', 'between', 'beyond', 'bicycle', 'bid',
    'bike', 'bind', 'biology', 'bird', 'birth', 'bitter', 'black', 'blade',
    'blame', 'blanket', 'blast', 'bleak', 'bless', 'blind', 'blood', 'blossom',
    'blow', 'blue', 'blur', 'blush', 'board', 'boat', 'body', 'boil',
    'bomb', 'bone', 'bonus', 'book', 'boost', 'border', 'boring', 'borrow',
    'boss', 'bottom', 'bounce', 'box', 'boy', 'bracket', 'brain', 'brand',
    'brass', 'brave', 'bread', 'breeze', 'brick', 'bridge', 'brief', 'bright',
    'bring', 'brisk', 'broccoli', 'broken', 'bronze', 'broom', 'brother', 'brown',
    'brush', 'bubble', 'buddy', 'budget', 'buffalo', 'build', 'bulb', 'bulk',
    'bullet', 'bundle', 'bunny', 'burden', 'burger', 'burst', 'bus', 'business',
    'busy', 'butter', 'buyer', 'buzz', 'cabbage', 'cabin', 'cable', 'cactus',
  ]

  const selected: string[] = []
  for (let i = 0; i < 12; i++) {
    const index = Math.floor(Math.random() * words.length)
    selected.push(words[index])
  }
  return selected.join(' ')
}
