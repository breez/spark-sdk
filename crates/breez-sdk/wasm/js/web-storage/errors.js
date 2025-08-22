/**
 * Storage Error Classes for Breez SDK Web Storage
 * ES6 module version - IndexedDB implementation
 */

export class StorageError extends Error {
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
