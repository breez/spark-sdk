/**
 * Top-level namespace for the Breez SDK Spark.
 *
 * Groups all static/global SDK functions that don't require a wallet
 * connection. Use `BreezSdkSpark.connect()` to obtain a `BreezSparkClient` instance.
 */
import {
  connect as _connect,
  connectWithSigner as _connectWithSigner,
  defaultConfig as _defaultConfig,
  defaultExternalSigner as _defaultExternalSigner,
  getSparkStatus as _getSparkStatus,
  initLogging as _initLogging,
  parse as _parse,
} from './generated/breez_sdk_spark';

import type {
  BreezSdkInterface,
  Config,
  ConnectRequest,
  ConnectWithSignerRequest,
  ExternalInputParser,
  ExternalSigner,
  InputType,
  KeySetConfig,
  Logger,
  Network,
  SparkStatus,
} from './generated/breez_sdk_spark';

/** Preferred name for the SDK client type. */
export type BreezSparkClient = BreezSdkInterface;

export class BreezSdkSpark {
  /** Returns a default SDK configuration for the given network. */
  static defaultConfig(network: Network): Config {
    return _defaultConfig(network);
  }

  /** Initializes the SDK logging subsystem. */
  static initLogging(
    logDir: string | undefined,
    appLogger: Logger | undefined,
    logFilter: string | undefined
  ): void {
    _initLogging(logDir, appLogger, logFilter);
  }

  /** Connects to the Spark network using the provided configuration and seed. */
  static async connect(request: ConnectRequest): Promise<BreezSparkClient> {
    return _connect(request);
  }

  /** Connects to the Spark network using an external signer. */
  static async connectWithSigner(
    request: ConnectWithSignerRequest
  ): Promise<BreezSparkClient> {
    return _connectWithSigner(request);
  }

  /** Creates a default external signer from a mnemonic phrase. */
  static defaultExternalSigner(
    mnemonic: string,
    passphrase: string | undefined,
    network: Network,
    keySetConfig: KeySetConfig | undefined
  ): ExternalSigner {
    return _defaultExternalSigner(mnemonic, passphrase, network, keySetConfig);
  }

  /** Parses a payment input string and returns the identified type. */
  static async parse(
    input: string,
    externalInputParsers?: ExternalInputParser[]
  ): Promise<InputType> {
    return _parse(input, externalInputParsers ?? null);
  }

  /** Fetches the current status of Spark network services. */
  static async getSparkStatus(): Promise<SparkStatus> {
    return _getSparkStatus();
  }
}
