// errors.cjs - Session manager error wrapper with cause chain support
class SessionManagerError extends Error {
    constructor(message, cause = null) {
        super(message);
        this.name = 'SessionManagerError';
        this.cause = cause;
        if (Error.captureStackTrace) {
            Error.captureStackTrace(this, SessionManagerError);
        }
    }
}

module.exports = { SessionManagerError };
