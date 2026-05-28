// errors.cjs - Session store error wrapper with cause chain support
class SessionStoreError extends Error {
    constructor(message, cause = null) {
        super(message);
        this.name = 'SessionStoreError';
        this.cause = cause;
        if (Error.captureStackTrace) {
            Error.captureStackTrace(this, SessionStoreError);
        }
    }
}

module.exports = { SessionStoreError };
