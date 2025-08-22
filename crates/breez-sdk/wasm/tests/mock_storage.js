/**
 * Mock Storage implementation for WASM tests
 * This provides a JavaScript implementation of the Storage interface
 * that can be used for testing the Rust WASM Storage wrapper
 */

export class MockStorage {
  constructor() {
    this.cachedItems = new Map();
    this.payments = new Map();
    this.unclaimedDeposits = new Map();
    this.depositRefunds = new Map();
    this.operationCount = 0;
  }

  _incrementOperationCount() {
    this.operationCount++;
  }

  // Cached Items
  getCachedItem(key) {
    this._incrementOperationCount();
    const value = this.cachedItems.get(key);
    return Promise.resolve(value !== undefined ? value : null);
  }

  setCachedItem(key, value) {
    this._incrementOperationCount();
    this.cachedItems.set(key, value);
    return Promise.resolve();
  }

  // Payments
  listPayments(offset = 0, limit = 100) {
    this._incrementOperationCount();
    const payments = Array.from(this.payments.values());
    return Promise.resolve(payments.slice(offset, offset + limit));
  }

  insertPayment(payment) {
    this._incrementOperationCount();
    if (!payment.id) {
      return Promise.reject(new Error("Payment must have an id"));
    }
    // Clone the payment to avoid mutations and convert BigInt values
    const cleanedPayment = {
      ...payment,
      amount: Number(payment.amount),
      fees: Number(payment.fees),
      timestamp: Number(payment.timestamp),
    };
    this.payments.set(payment.id, cleanedPayment);
    return Promise.resolve();
  }

  setPaymentMetadata(paymentId, metadata) {
    this._incrementOperationCount();
    const payment = this.payments.get(paymentId);
    if (!payment) {
      return Promise.reject(
        new Error(`Payment with id ${paymentId} not found`)
      );
    }

    // Apply metadata to payment details if it's a Lightning payment
    if (payment.details && payment.details.type === "lightning") {
      if (metadata.lnurlPayInfo) {
        payment.details.lnurlPayInfo = metadata.lnurlPayInfo;
      }
    }

    payment.metadata = metadata;
    return Promise.resolve();
  }

  getPaymentById(id) {
    this._incrementOperationCount();
    const payment = this.payments.get(id);
    if (!payment) {
      return Promise.reject(new Error(`Payment with id ${id} not found`));
    }
    // Return a clone to avoid mutations
    return Promise.resolve({ ...payment });
  }

  // Unclaimed Deposits
  addDeposit(txid, vout, amountSats) {
    this._incrementOperationCount();
    const key = `${txid}:${vout}`;
    this.unclaimedDeposits.set(key, {
      txid,
      vout,
      amountSats: Number(amountSats),
      refundTx: null,
      refundTxId: null,
      claimError: null,
    });
    return Promise.resolve();
  }

  deleteDeposit(txid, vout) {
    this._incrementOperationCount();
    const key = `${txid}:${vout}`;
    const result = this.unclaimedDeposits.delete(key);
    return Promise.resolve(result);
  }

  listDeposits() {
    this._incrementOperationCount();
    return Promise.resolve(Array.from(this.unclaimedDeposits.values()));
  }

  updateDeposit(txid, vout, payload) {
    this._incrementOperationCount();
    const key = `${txid}:${vout}`;
    const existingDeposit = this.unclaimedDeposits.get(key);
    if (!existingDeposit) {
      return Promise.reject(
        new Error(`Deposit with txid ${txid} and vout ${vout} not found`)
      );
    }

    // Update the existing deposit with the payload data
    const updatedDeposit = { ...existingDeposit };

    if (payload.type === "claimError") {
      updatedDeposit.claimError = payload.error;
      updatedDeposit.refundTx = null;
      updatedDeposit.refundTxId = null;
    } else if (payload.type === "refund") {
      updatedDeposit.refundTx = payload.refundTx;
      updatedDeposit.refundTxId = payload.refundTxid;
      updatedDeposit.claimError = null;
    }

    this.unclaimedDeposits.set(key, updatedDeposit);
    return Promise.resolve();
  }

  // Test utilities
  clear() {
    this.cachedItems.clear();
    this.payments.clear();
    this.unclaimedDeposits.clear();
    this.depositRefunds.clear();
    this.operationCount = 0;
  }

  getOperationCount() {
    return this.operationCount;
  }

  // Additional test utilities
  getCacheSize() {
    return this.cachedItems.size;
  }

  getPaymentsSize() {
    return this.payments.size;
  }

  getDepositsSize() {
    return this.unclaimedDeposits.size;
  }

  getRefundsSize() {
    return this.depositRefunds.size;
  }

  // Debug utilities
  dumpState() {
    return {
      cachedItems: Object.fromEntries(this.cachedItems),
      payments: Object.fromEntries(this.payments),
      unclaimedDeposits: Object.fromEntries(this.unclaimedDeposits),
      depositRefunds: Object.fromEntries(this.depositRefunds),
      operationCount: this.operationCount,
    };
  }

  // Simulate storage errors for error testing
  simulateError(operation, errorMessage = "Simulated storage error") {
    const originalMethod = this[operation];
    if (!originalMethod) {
      throw new Error(`Unknown operation: ${operation}`);
    }

    // Replace the method with one that returns a rejected promise
    this[operation] = () => {
      return Promise.reject(new Error(errorMessage));
    };

    // Return a function to restore the original method
    return () => {
      this[operation] = originalMethod;
    };
  }
}
