/**
 * Breez SDK CLI - React Native Terminal App
 *
 * A terminal/REPL-like UI that mirrors the same command structure as the Rust CLI.
 * Shows a setup screen on launch to select network, seed method, and passkey provider.
 * Then presents a scrollable output area at the top and a text input at the bottom.
 */

import 'react-native-get-random-values'
import React, { useState, useRef, useEffect, useCallback } from 'react'
import {
  Alert,
  SafeAreaView,
  ScrollView,
  TextInput,
  Text,
  View,
  StyleSheet,
  KeyboardAvoidingView,
  Platform,
  Share,
  StatusBar,
  TouchableOpacity,
} from 'react-native'
import {
  defaultConfig,
  Network,
  Seed,
  SdkBuilder,
  SdkEvent_Tags,
} from '@breeztech/breez-sdk-spark-react-native'
import type {
  BreezSdkInterface,
  TokenIssuerInterface,
  SdkEvent,
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

import { buildCommandRegistry, executeCommand } from './commands'
import { CliPersistence } from './persistence'
import { formatValue } from './serialization'
import {
  type PasskeyConfig,
  PasskeyProvider,
  buildPrfProvider,
  resolvePasskeySeed,
} from './passkey'

/** Base data directory. Network-specific subdirs are created below. */
const BASE_DATA_DIR = `${RNFS.DocumentDirectoryPath}/breez-cli-data`

/** Get a network-specific data directory to avoid storage conflicts. */
function getDataDir(network: Network): string {
  const suffix = network === Network.Mainnet ? 'mainnet' : 'regtest'
  return `${BASE_DATA_DIR}/${suffix}`
}

// ---------------------------------------------------------------------------
// Setup Screen
// ---------------------------------------------------------------------------

interface SetupConfig {
  network: Network
  passkeyConfig: PasskeyConfig | undefined
  /** If provided, use this mnemonic instead of generating/loading one. */
  restoreMnemonic: string | undefined
}

/**
 * Read the Breez API key from a local secrets file.
 * Create `secrets.json` in the project root with: { "apiKey": "your-key" }
 * This file is gitignored.
 */
let _cachedApiKey: string | undefined
function getApiKey(): string | undefined {
  if (_cachedApiKey !== undefined) return _cachedApiKey || undefined
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const secrets = require('../secrets.json')
    _cachedApiKey = secrets.apiKey ?? ''
    return _cachedApiKey || undefined
  } catch {
    _cachedApiKey = ''
    return undefined
  }
}

interface SetupScreenProps {
  onStart: (config: SetupConfig) => void
}

const SetupScreen: React.FC<SetupScreenProps> = ({ onStart }) => {
  const [network, setNetwork] = useState<Network>(Network.Regtest as Network)
  const [seedMethod, setSeedMethod] = useState<'mnemonic' | 'passkey'>('mnemonic')
  const [mnemonicMode, setMnemonicMode] = useState<'new' | 'restore'>('new')
  const [mnemonicInput, setMnemonicInput] = useState('')
  const [passkeyProvider, setPasskeyProvider] = useState<PasskeyProvider>(PasskeyProvider.File)

  const handleStart = () => {
    const restoreMnemonic = seedMethod === 'mnemonic' && mnemonicMode === 'restore'
      ? mnemonicInput.trim()
      : undefined
    if (restoreMnemonic && restoreMnemonic.split(/\s+/).length !== 12) {
      Alert.alert('Invalid Mnemonic', 'Please enter a valid 12-word recovery phrase.')
      return
    }
    onStart({
      network,
      passkeyConfig: seedMethod === 'passkey' ? {
        provider: passkeyProvider,
        label: 'Default',
        listLabels: false,
        storeLabel: false,
      } : undefined,
      restoreMnemonic,
    })
  }

  const handleShowDataInfo = async () => {
    try {
      const exists = await RNFS.exists(BASE_DATA_DIR)
      if (!exists) {
        Alert.alert('No Data', 'No CLI data directory found.')
        return
      }
      const items = await RNFS.readDir(BASE_DATA_DIR)
      const info = items.map(i => `${i.name} (${i.isDirectory() ? 'dir' : `${i.size}B`})`).join('\n')
      Alert.alert('CLI Data', `${BASE_DATA_DIR}\n\n${info}`)
    } catch (e) {
      Alert.alert('Error', String(e))
    }
  }

  const handleClearData = () => {
    Alert.alert(
      'Clear All Data',
      'This will delete all SDK storage, mnemonics, and passkey secrets for all networks. This cannot be undone.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Clear Everything',
          style: 'destructive',
          onPress: async () => {
            try {
              const exists = await RNFS.exists(BASE_DATA_DIR)
              if (exists) {
                await RNFS.unlink(BASE_DATA_DIR)
              }
              Alert.alert('Done', 'All CLI data cleared.')
            } catch (e) {
              Alert.alert('Error', String(e))
            }
          },
        },
      ],
    )
  }

  return (
    <SafeAreaView style={styles.container}>
      <StatusBar barStyle="light-content" backgroundColor="#1a1a2e" />
      <View style={styles.setupContainer}>
        <Text style={styles.setupTitle}>Breez SDK CLI</Text>
        <Text style={styles.setupSubtitle}>React Native</Text>

        {/* Network */}
        <Text style={styles.sectionLabel}>Network</Text>
        <View style={styles.buttonRow}>
          {([Network.Regtest, Network.Mainnet] as Network[]).map(n => (
            <TouchableOpacity
              key={String(n)}
              style={[styles.optionButton, network === n && styles.optionButtonActive]}
              onPress={() => setNetwork(n)}
            >
              <Text style={[styles.optionText, network === n && styles.optionTextActive]}>
                {n === Network.Mainnet ? 'Mainnet' : 'Regtest'}
              </Text>
            </TouchableOpacity>
          ))}
        </View>

        {/* Seed Method */}
        <Text style={styles.sectionLabel}>Seed Method</Text>
        <View style={styles.buttonRow}>
          <TouchableOpacity
            style={[styles.optionButton, seedMethod === 'mnemonic' && styles.optionButtonActive]}
            onPress={() => setSeedMethod('mnemonic')}
          >
            <Text style={[styles.optionText, seedMethod === 'mnemonic' && styles.optionTextActive]}>Mnemonic</Text>
          </TouchableOpacity>
          <TouchableOpacity
            style={[styles.optionButton, seedMethod === 'passkey' && styles.optionButtonActive]}
            onPress={() => setSeedMethod('passkey')}
          >
            <Text style={[styles.optionText, seedMethod === 'passkey' && styles.optionTextActive]}>Passkey</Text>
          </TouchableOpacity>
        </View>

        {/* Mnemonic Mode (only when mnemonic selected) */}
        {seedMethod === 'mnemonic' && (
          <>
            <Text style={styles.sectionLabel}>Wallet</Text>
            <View style={styles.buttonRow}>
              <TouchableOpacity
                style={[styles.optionButton, mnemonicMode === 'new' && styles.optionButtonActive]}
                onPress={() => setMnemonicMode('new')}
              >
                <Text style={[styles.optionText, mnemonicMode === 'new' && styles.optionTextActive]}>New Wallet</Text>
              </TouchableOpacity>
              <TouchableOpacity
                style={[styles.optionButton, mnemonicMode === 'restore' && styles.optionButtonActive]}
                onPress={() => setMnemonicMode('restore')}
              >
                <Text style={[styles.optionText, mnemonicMode === 'restore' && styles.optionTextActive]}>Restore</Text>
              </TouchableOpacity>
            </View>
            {mnemonicMode === 'restore' && (
              <TextInput
                style={styles.mnemonicInput}
                value={mnemonicInput}
                onChangeText={setMnemonicInput}
                placeholder="Enter 12-word recovery phrase..."
                placeholderTextColor="#555"
                multiline
                numberOfLines={3}
                autoCapitalize="none"
                autoCorrect={false}
              />
            )}
          </>
        )}

        {/* Passkey Provider */}
        {seedMethod === 'passkey' && (
          <>
            <Text style={styles.sectionLabel}>Passkey Provider</Text>
            <View style={styles.buttonRow}>
              <TouchableOpacity
                style={[styles.optionButton, passkeyProvider === PasskeyProvider.Platform && styles.optionButtonActive]}
                onPress={() => setPasskeyProvider(PasskeyProvider.Platform)}
              >
                <Text style={[styles.optionText, passkeyProvider === PasskeyProvider.Platform && styles.optionTextActive]}>Platform</Text>
              </TouchableOpacity>
              <TouchableOpacity
                style={[styles.optionButton, passkeyProvider === PasskeyProvider.File && styles.optionButtonActive]}
                onPress={() => setPasskeyProvider(PasskeyProvider.File)}
              >
                <Text style={[styles.optionText, passkeyProvider === PasskeyProvider.File && styles.optionTextActive]}>File</Text>
              </TouchableOpacity>
            </View>
            {passkeyProvider === PasskeyProvider.Platform && (
              <Text style={styles.hint}>
                Requires RP domain registration.{'\n'}
                Android 9+ with Google Play Services.
              </Text>
            )}
          </>
        )}

        {/* Start */}
        <TouchableOpacity style={styles.startButton} onPress={handleStart}>
          <Text style={styles.startButtonText}>START</Text>
        </TouchableOpacity>

        {/* Footer: Share Logs & Clear Data */}
        <View style={styles.setupFooter}>
          <TouchableOpacity style={styles.footerButton} onPress={handleShowDataInfo}>
            <Text style={styles.footerButtonText}>Data Info</Text>
          </TouchableOpacity>
          <TouchableOpacity style={styles.footerButton} onPress={handleClearData}>
            <Text style={[styles.footerButtonText, { color: '#e94560' }]}>Clear Data</Text>
          </TouchableOpacity>
        </View>
      </View>
    </SafeAreaView>
  )
}

// ---------------------------------------------------------------------------
// Event Listener
// ---------------------------------------------------------------------------

class CliEventListener {
  private appendLog: (text: string) => void

  constructor(appendLog: (text: string) => void) {
    this.appendLog = appendLog
  }

  onEvent = async (event: SdkEvent) => {
    let eventDesc = 'Unknown'
    if (event.tag === SdkEvent_Tags.Synced) {
      eventDesc = 'Synced'
    } else if (event.tag === SdkEvent_Tags.PaymentSucceeded) {
      eventDesc = 'PaymentSucceeded'
    } else if (event.tag === SdkEvent_Tags.PaymentPending) {
      eventDesc = 'PaymentPending'
    } else if (event.tag === SdkEvent_Tags.PaymentFailed) {
      eventDesc = 'PaymentFailed'
    } else if (event.tag === SdkEvent_Tags.ClaimedDeposits) {
      eventDesc = 'ClaimedDeposits'
    } else if (event.tag === SdkEvent_Tags.UnclaimedDeposits) {
      eventDesc = 'UnclaimedDeposits'
    } else if (event.tag === SdkEvent_Tags.NewDeposits) {
      eventDesc = 'NewDeposits'
    } else if (event.tag === SdkEvent_Tags.Optimization) {
      eventDesc = 'Optimization'
    } else if (event.tag === SdkEvent_Tags.LightningAddressChanged) {
      eventDesc = 'LightningAddressChanged'
    }
    this.appendLog(`[Event] ${eventDesc}: ${formatValue(event)}`)
  }
}

// ---------------------------------------------------------------------------
// CLI Screen
// ---------------------------------------------------------------------------

interface CliScreenProps {
  config: SetupConfig
  onDisconnect: () => void
}

const CliScreen: React.FC<CliScreenProps> = ({ config, onDisconnect }) => {
  const dataDir = getDataDir(config.network)
  const [logs, setLogs] = useState<string[]>([])
  const [inputText, setInputText] = useState('')
  const [isInitializing, setIsInitializing] = useState(true)
  const [isProcessing, setIsProcessing] = useState(false)

  const sdkRef = useRef<BreezSdkInterface | null>(null)
  const tokenIssuerRef = useRef<TokenIssuerInterface | null>(null)
  const registryRef = useRef(buildCommandRegistry())
  const persistenceRef = useRef(new CliPersistence(dataDir))
  const scrollViewRef = useRef<ScrollView>(null)
  const commandHistoryRef = useRef<string[]>([])

  const appendLog = useCallback((text: string) => {
    setLogs(prev => [...prev, text])
  }, [])

  const handleShareLogs = useCallback(async () => {
    try {
      await Share.share({
        title: 'Breez SDK CLI Session Logs',
        message: logs.join('\n'),
      })
    } catch {
      // User cancelled share
    }
  }, [logs])

  const disconnectAndGoBack = useCallback(async () => {
    if (sdkRef.current) {
      try {
        await sdkRef.current.disconnect()
      } catch {
        // Ignore disconnect errors
      }
      sdkRef.current = null
      tokenIssuerRef.current = null
    }
    onDisconnect()
  }, [onDisconnect])

  // Initialize the SDK on mount
  useEffect(() => {
    let mounted = true

    const initSdk = async () => {
      try {
        appendLog('Breez SDK CLI Interactive Mode')
        appendLog('Initializing SDK...')

        const persistence = persistenceRef.current
        const sdkConfig = defaultConfig(config.network)
        const apiKey = getApiKey()
        if (apiKey) {
          sdkConfig.apiKey = apiKey
          appendLog('API key loaded from secrets.json')
        } else if (config.network === Network.Mainnet) {
          appendLog('Warning: No API key found. Create secrets.json with { "apiKey": "..." }')
        }

        let seed: Seed

        if (config.passkeyConfig) {
          appendLog(`Using passkey provider: ${config.passkeyConfig.provider}`)

          const prfProvider = await buildPrfProvider(config.passkeyConfig.provider, dataDir)
          const breezApiKey = sdkConfig.apiKey ?? undefined

          const result = await resolvePasskeySeed(
            prfProvider,
            breezApiKey,
            config.passkeyConfig.label,
            config.passkeyConfig.listLabels,
            config.passkeyConfig.storeLabel,
          )

          if (result.labels && result.labels.length > 0) {
            appendLog('Available labels:')
            for (let i = 0; i < result.labels.length; i++) {
              appendLog(`  ${i + 1}: ${result.labels[i]}`)
            }
          }

          seed = result.seed
          appendLog('Passkey seed derived successfully')
        } else if (config.restoreMnemonic) {
          appendLog('Restoring from provided mnemonic')
          seed = new Seed.Mnemonic({ mnemonic: config.restoreMnemonic, passphrase: undefined })
        } else {
          const mnemonic = await persistence.getOrCreateMnemonic()
          seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })
        }

        const builder = new SdkBuilder(sdkConfig, seed)
        await builder.withDefaultStorage(dataDir)

        const sdk = await builder.build()
        const tokenIssuer = sdk.getTokenIssuer()

        const listener = new CliEventListener(appendLog)
        await sdk.addEventListener(listener)

        if (mounted) {
          sdkRef.current = sdk
          tokenIssuerRef.current = tokenIssuer

          const networkLabel = config.network === Network.Mainnet ? 'mainnet' : 'regtest'
          appendLog(`SDK initialized on ${networkLabel}`)
          appendLog(`Data dir: ${dataDir}`)
          appendLog("Type 'help' for available commands.")
          appendLog("Use the \u2190 button in the header to disconnect and go back.")
          appendLog('')
          setIsInitializing(false)
        }

        const history = await persistence.getHistory()
        commandHistoryRef.current = history
      } catch (error: unknown) {
        const message = error instanceof Error ? error.message : String(error)
        if (mounted) {
          appendLog(`SDK initialization failed: ${message}`)
          appendLog('')
          appendLog('Tap the \u2190 button to go back and try again.')
          setIsInitializing(false)
        }
      }
    }

    initSdk()

    return () => {
      mounted = false
      if (sdkRef.current) {
        sdkRef.current.disconnect().catch(() => {})
      }
    }
  }, [appendLog, config, dataDir])

  useEffect(() => {
    const timer = setTimeout(() => {
      scrollViewRef.current?.scrollToEnd({ animated: true })
    }, 50)
    return () => clearTimeout(timer)
  }, [logs])

  const handleSubmit = useCallback(async () => {
    const trimmed = inputText.trim()
    if (trimmed === '' || isProcessing) return

    const sdk = sdkRef.current
    const tokenIssuer = tokenIssuerRef.current
    const registry = registryRef.current
    const persistence = persistenceRef.current

    const networkLabel = config.network === Network.Mainnet ? 'mainnet' : 'regtest'
    appendLog(`breez-spark-cli [${networkLabel}]> ${trimmed}`)
    setInputText('')
    commandHistoryRef.current.push(trimmed)
    persistence.addToHistory(trimmed).catch(() => {})

    // Handle 'exit' / 'quit' locally — disconnect and go back
    if (trimmed === 'exit' || trimmed === 'quit') {
      appendLog('Disconnecting...')
      await disconnectAndGoBack()
      return
    }

    if (!sdk || !tokenIssuer) {
      appendLog('Error: SDK not initialized')
      return
    }

    setIsProcessing(true)

    try {
      const { output, shouldContinue } = await executeCommand(
        trimmed,
        sdk,
        tokenIssuer,
        registry
      )

      if (output) {
        appendLog(output)
      }

      if (!shouldContinue) {
        await disconnectAndGoBack()
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error)
      appendLog(`Error: ${message}`)
    } finally {
      setIsProcessing(false)
    }
  }, [inputText, isProcessing, appendLog, config, disconnectAndGoBack])

  const networkLabel = config.network === Network.Mainnet ? 'mainnet' : 'regtest'
  const prompt = isInitializing
    ? 'initializing...'
    : isProcessing
      ? 'processing...'
      : `breez-spark-cli [${networkLabel}]>`

  return (
    <SafeAreaView style={styles.container}>
      <StatusBar barStyle="light-content" backgroundColor="#1a1a2e" />
      <KeyboardAvoidingView
        style={styles.keyboardView}
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
        keyboardVerticalOffset={Platform.OS === 'ios' ? 0 : 20}
      >
        {/* Header with back button and share */}
        <View style={styles.header}>
          <TouchableOpacity onPress={disconnectAndGoBack} style={styles.backButton}>
            <Text style={styles.backButtonText}>{'\u2190'}</Text>
          </TouchableOpacity>
          <View style={styles.headerTextContainer}>
            <Text style={styles.headerText}>Breez SDK CLI</Text>
            <Text style={styles.headerSubtext}>
              {isInitializing ? 'Initializing...' : `Connected (${networkLabel})`}
            </Text>
          </View>
          <TouchableOpacity onPress={handleShareLogs} style={styles.shareButton}>
            <Text style={styles.shareButtonText}>Share</Text>
          </TouchableOpacity>
        </View>

        {/* Output Log */}
        <ScrollView
          ref={scrollViewRef}
          style={styles.logContainer}
          contentContainerStyle={styles.logContent}
        >
          {logs.map((line, index) => (
            <Text key={index} style={styles.logLine} selectable>
              {line}
            </Text>
          ))}
        </ScrollView>

        {/* Input Area */}
        <View style={styles.inputContainer}>
          <Text style={styles.prompt}>{prompt} </Text>
          <TextInput
            style={styles.input}
            value={inputText}
            onChangeText={setInputText}
            onSubmitEditing={handleSubmit}
            placeholder="Type a command..."
            placeholderTextColor="#666"
            autoCapitalize="none"
            autoCorrect={false}
            spellCheck={false}
            editable={!isInitializing}
            returnKeyType="send"
            blurOnSubmit={false}
          />
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
  )
}

// ---------------------------------------------------------------------------
// Root App
// ---------------------------------------------------------------------------

const App: React.FC = () => {
  const [config, setConfig] = useState<SetupConfig | null>(null)

  if (!config) {
    return <SetupScreen onStart={setConfig} />
  }

  return <CliScreen config={config} onDisconnect={() => setConfig(null)} />
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#1a1a2e',
  },
  keyboardView: {
    flex: 1,
  },
  // Setup screen
  setupContainer: {
    flex: 1,
    paddingHorizontal: 32,
    paddingTop: 60,
  },
  setupTitle: {
    color: '#e94560',
    fontSize: 28,
    fontWeight: 'bold',
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    textAlign: 'center',
  },
  setupSubtitle: {
    color: '#53af7b',
    fontSize: 14,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    textAlign: 'center',
    marginBottom: 40,
  },
  sectionLabel: {
    color: '#888',
    fontSize: 12,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    textTransform: 'uppercase',
    letterSpacing: 1,
    marginTop: 24,
    marginBottom: 8,
  },
  buttonRow: {
    flexDirection: 'row',
    gap: 10,
  },
  optionButton: {
    flex: 1,
    paddingVertical: 12,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: '#333',
    alignItems: 'center',
  },
  optionButtonActive: {
    borderColor: '#e94560',
    backgroundColor: 'rgba(233, 69, 96, 0.15)',
  },
  optionText: {
    color: '#666',
    fontSize: 14,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    fontWeight: '600',
  },
  optionTextActive: {
    color: '#e94560',
  },
  hint: {
    color: '#666',
    fontSize: 11,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    marginTop: 8,
    lineHeight: 16,
  },
  mnemonicInput: {
    marginTop: 12,
    borderWidth: 1,
    borderColor: '#333',
    borderRadius: 8,
    padding: 12,
    color: '#fff',
    fontSize: 13,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    minHeight: 70,
    textAlignVertical: 'top',
  },
  startButton: {
    marginTop: 40,
    paddingVertical: 16,
    borderRadius: 8,
    backgroundColor: '#e94560',
    alignItems: 'center',
  },
  startButtonText: {
    color: '#fff',
    fontSize: 16,
    fontWeight: 'bold',
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    letterSpacing: 2,
  },
  setupFooter: {
    flexDirection: 'row',
    justifyContent: 'center',
    gap: 24,
    marginTop: 32,
    paddingBottom: 16,
  },
  footerButton: {
    paddingVertical: 8,
    paddingHorizontal: 16,
  },
  footerButtonText: {
    color: '#888',
    fontSize: 13,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    textDecorationLine: 'underline',
  },
  // CLI screen
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 12,
    backgroundColor: '#16213e',
    borderBottomWidth: 1,
    borderBottomColor: '#0f3460',
  },
  backButton: {
    paddingRight: 12,
    paddingVertical: 4,
  },
  backButtonText: {
    color: '#e94560',
    fontSize: 22,
    fontWeight: 'bold',
  },
  headerTextContainer: {
    flex: 1,
  },
  shareButton: {
    paddingLeft: 12,
    paddingVertical: 4,
  },
  shareButtonText: {
    color: '#53af7b',
    fontSize: 13,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
  },
  headerText: {
    color: '#e94560',
    fontSize: 18,
    fontWeight: 'bold',
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
  },
  headerSubtext: {
    color: '#53af7b',
    fontSize: 12,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    marginTop: 2,
  },
  logContainer: {
    flex: 1,
    backgroundColor: '#1a1a2e',
  },
  logContent: {
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  logLine: {
    color: '#c0c0c0',
    fontSize: 13,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    lineHeight: 20,
  },
  inputContainer: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 12,
    paddingVertical: 8,
    backgroundColor: '#16213e',
    borderTopWidth: 1,
    borderTopColor: '#0f3460',
  },
  prompt: {
    color: '#53af7b',
    fontSize: 13,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
  },
  input: {
    flex: 1,
    color: '#ffffff',
    fontSize: 13,
    fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace',
    paddingVertical: 4,
    paddingHorizontal: 4,
  },
})

export default App
