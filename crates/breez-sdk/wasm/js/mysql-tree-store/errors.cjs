class TreeStoreError extends Error {
    constructor(message, cause = null) {
        super(message);
        this.name = 'TreeStoreError';
        this.cause = cause;
    }
}

module.exports = { TreeStoreError };
