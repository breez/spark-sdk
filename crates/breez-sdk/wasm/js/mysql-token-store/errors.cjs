class TokenStoreError extends Error {
    constructor(message, cause = null) {
        super(message);
        this.name = 'TokenStoreError';
        this.cause = cause;
    }
}

module.exports = { TokenStoreError };
