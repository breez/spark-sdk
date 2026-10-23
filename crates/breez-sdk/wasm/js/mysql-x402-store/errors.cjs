// errors.cjs - x402 store error wrapper with cause chain support
class X402StoreError extends Error {
  constructor(message, cause = null) {
    super(message);
    this.name = "X402StoreError";
    this.cause = cause;
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, X402StoreError);
    }
  }
}

module.exports = { X402StoreError };
