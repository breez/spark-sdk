// errors.cjs - Tree store error wrapper with cause chain support
class TreeStoreError extends Error {
    constructor(message, cause = null) {
        super(message);
        this.name = 'TreeStoreError';
        this.cause = cause;
        if (Error.captureStackTrace) {
            Error.captureStackTrace(this, TreeStoreError);
        }
    }
}

module.exports = { TreeStoreError };
