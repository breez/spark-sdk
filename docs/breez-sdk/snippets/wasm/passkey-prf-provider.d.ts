// Type declarations for the passkey-prf-provider subpath export.
// The actual implementation ships at @breeztech/breez-sdk-spark/passkey-prf-provider.
declare module '@breeztech/breez-sdk-spark/passkey-prf-provider' {
  export class WebAuthnPrfProvider {
    constructor (options?: {
      rpId?: string
      rpName?: string
      userName?: string
      userDisplayName?: string
    })
    derivePrfSeed (salt: string): Promise<Uint8Array>
    isPrfAvailable (): Promise<boolean>
    createPasskey (): Promise<void>
  }
}
