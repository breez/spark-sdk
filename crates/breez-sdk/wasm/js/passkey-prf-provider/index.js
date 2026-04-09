/**
 * WebAuthn PRF provider for browser environments.
 *
 * Implements the PasskeyPrfProvider interface using the WebAuthn API
 * with the PRF extension (navigator.credentials.create/get).
 *
 * Uses discoverable credentials (resident keys) so no credential storage is needed.
 * The credential lives on the authenticator and is discovered by rpId.
 *
 * On first use, if no credential exists for the RP ID, a new passkey is
 * automatically created (registered), then the assertion is retried.
 *
 * @example
 * ```javascript
 * import { WebAuthnPrfProvider, Passkey } from '@breeztech/breez-sdk-spark'
 *
 * const prfProvider = new WebAuthnPrfProvider()
 * const passkey = new Passkey(prfProvider, undefined)
 * const wallet = await passkey.getWallet('personal')
 * ```
 */

const DEFAULT_RP_ID = 'keys.breez.technology';
const DEFAULT_RP_NAME = 'Breez SDK';

/**
 * Generate cryptographically random bytes.
 * @param {number} length
 * @returns {Uint8Array}
 */
function randomBytes(length) {
    const bytes = new Uint8Array(length);
    crypto.getRandomValues(bytes);
    return bytes;
}

/**
 * WebAuthn-based PRF provider for browser environments.
 */
export class WebAuthnPrfProvider {
    /**
     * @param {object} [options]
     * @param {string} [options.rpId='keys.breez.technology'] - Relying Party ID.
     *   Must match the domain configured in .well-known/webauthn for cross-platform
     *   credential sharing. Changing this after users have registered passkeys will
     *   make their existing credentials undiscoverable — they would need to create
     *   new passkeys with the new RP ID.
     * @param {string} [options.rpName='Breez SDK'] - RP display name shown during
     *   credential registration. Only used when creating new passkeys; changing it
     *   does not affect existing credentials.
     * @param {string} [options.userName] - User name stored with the credential,
     *   shown as a secondary label in some passkey managers. Defaults to rpName.
     *   Only used during registration; changing it does not affect existing credentials.
     * @param {string} [options.userDisplayName] - User display name shown as the
     *   primary label in the passkey picker. Defaults to userName. Only used during
     *   registration; changing it does not affect existing credentials.
     */
    constructor(options = {}) {
        this.rpId = options.rpId || DEFAULT_RP_ID;
        this.rpName = options.rpName || DEFAULT_RP_NAME;
        this.userName = options.userName || this.rpName;
        this.userDisplayName = options.userDisplayName || this.userName;
    }

    /**
     * Derive a 32-byte seed from passkey PRF with the given salt.
     *
     * Authenticates the user via a platform passkey and evaluates the PRF extension.
     * If no credential exists for this RP ID, a new passkey is created automatically.
     *
     * @param {string} salt - The salt string to use for PRF evaluation.
     * @returns {Promise<Uint8Array>} The 32-byte PRF output.
     * @throws {Error} If authentication fails, PRF is not supported, or the user cancels.
     */
    async derivePrfSeed(salt) {
        const saltBytes = new TextEncoder().encode(salt);

        // Try assertion first (existing credential)
        try {
            return await this._getAssertionWithPrf(saltBytes);
        } catch (error) {
            // If no credential found, register a new one and retry
            if (this._isNoCredentialError(error)) {
                await this._registerCredential();
                return await this._getAssertionWithPrf(saltBytes);
            }
            throw error;
        }
    }

    /**
     * Create a new passkey with PRF support.
     *
     * Only registers the credential — no seed derivation. Triggers exactly
     * 1 WebAuthn prompt. Use this to separate credential creation from
     * derivation in multi-step onboarding flows.
     *
     * If a passkey already exists for this RP ID, this will create an
     * additional credential (browsers allow multiple per RP).
     *
     * @returns {Promise<void>}
     * @throws {Error} If the user cancels or PRF is not supported by the authenticator.
     */
    async createPasskey() {
        await this._registerCredential();
    }

    /**
     * Check if a PRF-capable passkey is available on this device.
     *
     * @returns {Promise<boolean>} true if WebAuthn with PRF extension is likely supported.
     */
    async isPrfAvailable() {
        try {
            if (typeof window === 'undefined' || !window.PublicKeyCredential) {
                return false;
            }
            if (typeof PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable !== 'function') {
                return false;
            }
            return await PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();
        } catch {
            return false;
        }
    }

    /**
     * Perform a WebAuthn assertion with PRF extension.
     * @param {Uint8Array} saltBytes
     * @returns {Promise<Uint8Array>}
     * @private
     */
    async _getAssertionWithPrf(saltBytes) {
        const options = {
            publicKey: {
                challenge: randomBytes(32),
                rpId: this.rpId,
                allowCredentials: [], // discoverable credentials
                userVerification: 'required',
                extensions: {
                    prf: {
                        eval: {
                            first: saltBytes,
                        },
                    },
                },
            },
        };

        let credential;
        try {
            credential = await navigator.credentials.get(options);
        } catch (error) {
            throw this._mapError(error);
        }

        if (!credential) {
            throw new Error('Credential not found');
        }

        const extensionResults = credential.getClientExtensionResults();

        if (!extensionResults.prf || !extensionResults.prf.results || !extensionResults.prf.results.first) {
            throw new Error('PRF not supported by authenticator');
        }

        return new Uint8Array(extensionResults.prf.results.first);
    }

    /**
     * Register a new discoverable credential with PRF extension enabled.
     * @returns {Promise<void>}
     * @private
     */
    async _registerCredential() {
        const options = {
            publicKey: {
                challenge: randomBytes(32),
                rp: {
                    id: this.rpId,
                    name: this.rpName,
                },
                user: {
                    id: randomBytes(16),
                    name: this.userName,
                    displayName: this.userDisplayName,
                },
                pubKeyCredParams: [
                    { type: 'public-key', alg: -7 },   // ES256 (P-256)
                    { type: 'public-key', alg: -257 },  // RS256
                ],
                authenticatorSelection: {
                    residentKey: 'required',
                    requireResidentKey: true,
                    userVerification: 'required',
                },
                extensions: {
                    prf: {},
                },
            },
        };

        let credential;
        try {
            credential = await navigator.credentials.create(options);
        } catch (error) {
            throw this._mapError(error);
        }

        if (!credential) {
            throw new Error('Credential creation failed');
        }

        // Verify PRF extension was acknowledged
        const extensionResults = credential.getClientExtensionResults();
        if (!extensionResults.prf || !extensionResults.prf.enabled) {
            throw new Error('PRF not supported by authenticator');
        }
    }

    /**
     * Check if the error indicates no credential was found.
     * @param {Error} error
     * @returns {boolean}
     * @private
     */
    _isNoCredentialError(error) {
        if (!error) return false;
        const message = error.message || '';
        // Browsers throw NotAllowedError when user cancels or no credential is available.
        // Some browsers distinguish these; we treat "no credentials" as recoverable.
        // The message varies by browser, so check common patterns.
        return (
            message.includes('Credential not found') ||
            message.includes('no credentials') ||
            message.includes('No credentials') ||
            message.includes('empty allowCredentials') ||
            // Chrome-specific: no credential available for the given RP
            (error.name === 'NotAllowedError' && message.includes('not allowed'))
        );
    }

    /**
     * Map WebAuthn errors to descriptive error messages.
     * @param {Error} error
     * @returns {Error}
     * @private
     */
    _mapError(error) {
        if (!error) {
            return new Error('Unknown WebAuthn error');
        }

        switch (error.name) {
            case 'NotAllowedError':
                // Could be user cancellation or no credentials
                if (error.message && (
                    error.message.includes('cancelled') ||
                    error.message.includes('canceled') ||
                    error.message.includes('denied') ||
                    error.message.includes('The operation either timed out or was not allowed')
                )) {
                    return new Error('User cancelled authentication');
                }
                return error;

            case 'SecurityError':
                return new Error(`Authentication failed: ${error.message}`);

            case 'InvalidStateError':
                return new Error(`Authentication failed: ${error.message}`);

            case 'AbortError':
                return new Error('User cancelled authentication');

            default:
                return error;
        }
    }
}
