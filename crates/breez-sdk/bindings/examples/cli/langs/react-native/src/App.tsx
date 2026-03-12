/**
 * Breez SDK CLI - React Native Terminal App
 *
 * A terminal/REPL-like UI that mirrors the same command structure as the Rust CLI.
 * The screen has a scrollable output area at the top and a text input at the bottom.
 *
 * Supports passkey-based seed derivation (matching the Rust CLI's --passkey flag)
 * as well as traditional mnemonic-based seed.
 */

import 'react-native-get-random-values'
import React, { useState, useRef, useEffect, useCallback } from 'react'
import {
  SafeAreaView,
  ScrollView,
  TextInput,
  Text,
  View,
  StyleSheet,
  KeyboardAvoidingView,
  Platform,
  StatusBar,
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

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/** Default network for the CLI app. Change to Network.Mainnet for production. */
const DEFAULT_NETWORK = Network.Regtest as Network

/** Data directory for the SDK. Uses the app's document directory. */
const DATA_DIR = `${RNFS.DocumentDirectoryPath}/breez-cli-data`

/**
 * Passkey configuration. Set to undefined for mnemonic-based seed (default).
 *
 * To enable passkey support, set this to a PasskeyConfig object. Example:
 *
 *   const PASSKEY_CONFIG: PasskeyConfig = {
 *     provider: PasskeyProvider.File,
 *     label: 'personal',
 *     listLabels: false,
 *     storeLabel: false,
 *   }
 *
 * In a production app, these would come from a settings screen or launch config.
 */
const PASSKEY_CONFIG = undefined as PasskeyConfig | undefined

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
    } else if (event.tag === SdkEvent_Tags.Optimization) {
      eventDesc = 'Optimization'
    }
    this.appendLog(`[Event] ${eventDesc}: ${formatValue(event)}`)
  }
}

// ---------------------------------------------------------------------------
// App Component
// ---------------------------------------------------------------------------

const App: React.FC = () => {
  const [logs, setLogs] = useState<string[]>([])
  const [inputText, setInputText] = useState('')
  const [isInitializing, setIsInitializing] = useState(true)
  const [isProcessing, setIsProcessing] = useState(false)

  const sdkRef = useRef<BreezSdkInterface | null>(null)
  const tokenIssuerRef = useRef<TokenIssuerInterface | null>(null)
  const registryRef = useRef(buildCommandRegistry())
  const persistenceRef = useRef(new CliPersistence(DATA_DIR))
  const scrollViewRef = useRef<ScrollView>(null)
  const commandHistoryRef = useRef<string[]>([])

  // Append a log line and auto-scroll
  const appendLog = useCallback((text: string) => {
    setLogs(prev => [...prev, text])
  }, [])

  // Initialize the SDK on mount
  useEffect(() => {
    let mounted = true

    const initSdk = async () => {
      try {
        appendLog('Breez SDK CLI Interactive Mode')
        appendLog('Initializing SDK...')

        const persistence = persistenceRef.current

        const config = defaultConfig(DEFAULT_NETWORK)
        // API key can be set via environment or hardcoded for testing
        // config.apiKey = '<your-api-key>'

        let seed: Seed

        if (PASSKEY_CONFIG) {
          appendLog(`Using passkey provider: ${PASSKEY_CONFIG.provider}`)

          const prfProvider = await buildPrfProvider(PASSKEY_CONFIG.provider, DATA_DIR)

          // Retrieve the Breez API key if set
          const breezApiKey = config.apiKey ?? undefined

          const result = await resolvePasskeySeed(
            prfProvider,
            breezApiKey,
            PASSKEY_CONFIG.label,
            PASSKEY_CONFIG.listLabels,
            PASSKEY_CONFIG.storeLabel,
          )

          if (result.labels && result.labels.length > 0) {
            appendLog('Available labels:')
            for (let i = 0; i < result.labels.length; i++) {
              appendLog(`  ${i + 1}: ${result.labels[i]}`)
            }
          }

          if (PASSKEY_CONFIG.storeLabel && PASSKEY_CONFIG.label) {
            appendLog(`Label '${PASSKEY_CONFIG.label}' published to Nostr`)
          }

          seed = result.seed
          appendLog('Passkey seed derived successfully')
        } else {
          const mnemonic = await persistence.getOrCreateMnemonic()
          seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })
        }

        const builder = new SdkBuilder(config, seed)
        await builder.withDefaultStorage(DATA_DIR)

        const sdk = await builder.build()
        const tokenIssuer = sdk.getTokenIssuer()

        // Add event listener
        const listener = new CliEventListener(appendLog)
        await sdk.addEventListener(listener)

        if (mounted) {
          sdkRef.current = sdk
          tokenIssuerRef.current = tokenIssuer

          const networkLabel = DEFAULT_NETWORK === Network.Mainnet ? 'mainnet' : 'regtest'
          appendLog(`SDK initialized on ${networkLabel}`)
          appendLog("Type 'help' for available commands or 'exit' to quit")
          appendLog('')
          setIsInitializing(false)
        }

        // Load command history
        const history = await persistence.getHistory()
        commandHistoryRef.current = history
      } catch (error: unknown) {
        const message = error instanceof Error ? error.message : String(error)
        if (mounted) {
          appendLog(`SDK initialization failed: ${message}`)
          setIsInitializing(false)
        }
      }
    }

    initSdk()

    return () => {
      mounted = false
      // Disconnect on unmount
      if (sdkRef.current) {
        sdkRef.current.disconnect().catch(() => {
          // Ignore disconnect errors on cleanup
        })
      }
    }
  }, [appendLog])

  // Auto-scroll to bottom when logs change
  useEffect(() => {
    const timer = setTimeout(() => {
      scrollViewRef.current?.scrollToEnd({ animated: true })
    }, 50)
    return () => clearTimeout(timer)
  }, [logs])

  // Handle command submission
  const handleSubmit = useCallback(async () => {
    const trimmed = inputText.trim()
    if (trimmed === '' || isProcessing) return

    const sdk = sdkRef.current
    const tokenIssuer = tokenIssuerRef.current
    const registry = registryRef.current
    const persistence = persistenceRef.current

    const networkLabel = DEFAULT_NETWORK === Network.Mainnet ? 'mainnet' : 'regtest'
    appendLog(`breez-spark-cli [${networkLabel}]> ${trimmed}`)
    setInputText('')
    // Save to history
    commandHistoryRef.current.push(trimmed)
    persistence.addToHistory(trimmed).catch(() => {
      // Ignore history save errors
    })

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
        try {
          await sdk.disconnect()
        } catch {
          // Ignore disconnect errors
        }
        sdkRef.current = null
        tokenIssuerRef.current = null
        appendLog('SDK disconnected.')
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error)
      appendLog(`Error: ${message}`)
    } finally {
      setIsProcessing(false)
    }
  }, [inputText, isProcessing, appendLog])

  // Get the prompt prefix
  const networkLabel = DEFAULT_NETWORK === Network.Mainnet ? 'mainnet' : 'regtest'
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
        {/* Header */}
        <View style={styles.header}>
          <Text style={styles.headerText}>Breez SDK CLI</Text>
          <Text style={styles.headerSubtext}>
            {isInitializing ? 'Initializing...' : `Connected (${networkLabel})`}
          </Text>
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
  header: {
    paddingHorizontal: 16,
    paddingVertical: 12,
    backgroundColor: '#16213e',
    borderBottomWidth: 1,
    borderBottomColor: '#0f3460',
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
