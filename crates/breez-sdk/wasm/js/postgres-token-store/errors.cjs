// errors.cjs - Token store error wrapper with cause chain support
class TokenStoreError extends Error {
    constructor(message, cause = null) {
        super(message);
        this.name = 'TokenStoreError';
        this.cause = cause;
        if (Error.captureStackTrace) {
            Error.captureStackTrace(this, TokenStoreError);
        }
    }
}

module.exports = { TokenStoreError };
