package breez_sdk_spark

/** Preferred name for the SDK client type. */
typealias BreezSparkClient = BreezSdk

/**
 * Top-level namespace for the Breez SDK Spark.
 *
 * Groups all static/global SDK functions that don't require a wallet
 * connection. Use `BreezSdkSpark.connect()` to obtain a [BreezSparkClient] instance.
 */
object BreezSdkSpark {
    /** Returns a default SDK configuration for the given network. */
    fun defaultConfig(network: Network): Config =
        breez_sdk_spark.defaultConfig(network)

    /** Initializes the SDK logging subsystem. */
    fun initLogging(logDir: String? = null, appLogger: Logger? = null, logFilter: String? = null) =
        breez_sdk_spark.initLogging(logDir, appLogger, logFilter)

    /** Connects to the Spark network using the provided configuration and seed. */
    suspend fun connect(request: ConnectRequest): BreezSparkClient =
        breez_sdk_spark.connect(request)

    /** Connects to the Spark network using an external signer. */
    suspend fun connectWithSigner(request: ConnectWithSignerRequest): BreezSparkClient =
        breez_sdk_spark.connectWithSigner(request)

    /** Creates a default external signer from a mnemonic phrase. */
    fun defaultExternalSigner(
        mnemonic: String,
        passphrase: String? = null,
        network: Network,
        keySetConfig: KeySetConfig? = null,
    ): ExternalSigner =
        breez_sdk_spark.defaultExternalSigner(mnemonic, passphrase, network, keySetConfig)

    /** Parses a payment input string and returns the identified type. */
    suspend fun parse(
        input: String,
        externalInputParsers: List<ExternalInputParser>? = null,
    ): InputType =
        breez_sdk_spark.parse(input, externalInputParsers)

    /** Fetches the current status of Spark network services. */
    suspend fun getSparkStatus(): SparkStatus =
        breez_sdk_spark.getSparkStatus()
}
