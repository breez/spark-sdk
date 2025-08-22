/**
 * Storage Error Classes for Breez SDK Node.js Storage
 * CommonJS version
 */

class StorageError extends Error {
    constructor(message, cause = null) {
        super(message);
        this.name = 'StorageError';
        this.cause = cause;
        
        // Maintain proper stack trace for where our error was thrown (only available on V8)
        if (Error.captureStackTrace) {
            Error.captureStackTrace(this, StorageError);
        }
    }
}

module.exports = { StorageError };
