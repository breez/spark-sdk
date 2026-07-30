/**
 * ES module implementation of the durable tree store for browsers, backed by
 * IndexedDB. Ports the two-table (leaves + ancestors) model and reservation /
 * refresh / spent-guard semantics of the PostgreSQL tree store to a single-user
 * IndexedDB database: no user_id scoping and no `brz_` table prefix, since each
 * browser origin has its own isolated database.
 *
 * IndexedDB transaction lifetime: a transaction auto-commits once its request
 * queue drains and control returns to the event loop, so awaiting any non-IDB
 * promise mid-transaction closes it. Every logical operation therefore runs in
 * ONE transaction: all reads are issued up front, and the compute + writes run
 * synchronously inside the last read's success handler (never awaiting anything
 * that isn't part of the transaction). Because IndexedDB serializes readwrite
 * transactions with overlapping store scope, this also gives the mutating ops
 * the same serialization the Postgres backend gets from its per-tenant lock.
 */

/** Reservations older than this are stale and get released on the next refresh. */
const RESERVATION_TIMEOUT_MS = 300 * 1000; // 5 minutes

/** Spent-leaf markers older than this (relative to a refresh) are pruned. */
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

const DB_VERSION = 5;

const STORE_LEAVES = "leaves";
const STORE_ANCESTORS = "ancestors";
const STORE_RESERVATIONS = "reservations";
const STORE_SPENT = "spent";
const STORE_SWAP_STATUS = "swapStatus";

/** Singleton key of the swap-status row. */
const SWAP_STATUS_ID = 1;

class TreeStoreError extends Error {
  constructor(message, cause = null) {
    super(message);
    this.name = "TreeStoreError";
    this.cause = cause;
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, TreeStoreError);
    }
  }
}

function _resolveIndexedDB() {
  if (typeof indexedDB !== "undefined") return indexedDB;
  if (typeof globalThis !== "undefined" && globalThis.indexedDB) {
    return globalThis.indexedDB;
  }
  if (typeof self !== "undefined" && self.indexedDB) return self.indexedDB;
  if (typeof window !== "undefined" && window.indexedDB) return window.indexedDB;
  return null;
}

/**
 * Pair a leaf with its ancestors (nearest first) by walking `parent_node_id`
 * through `nodes`. Returns null if the leaf itself is absent; stops at a gap or
 * cycle, returning a partial chain.
 * @param {Map<string, object>} nodes
 * @param {string} leafId
 * @returns {{leaf: object, ancestors: Array<object>}|null}
 */
function assembleExitChain(nodes, leafId) {
  const leaf = nodes.get(leafId);
  if (!leaf) return null;
  const ancestors = [];
  const visited = new Set([leafId]);
  let current = leaf.parent_node_id;
  while (current != null && !visited.has(current)) {
    visited.add(current);
    const node = nodes.get(current);
    if (!node) break;
    ancestors.push(node);
    current = node.parent_node_id;
  }
  return { leaf, ancestors };
}

class WebTreeStore {
  constructor(dbName = "BreezSdkSparkTree", logger = null) {
    this.dbName = dbName;
    this.db = null;
    this.logger = logger;
  }

  async initialize() {
    if (this.db) return this;

    const idbFactory = _resolveIndexedDB();
    if (!idbFactory) {
      throw new TreeStoreError("IndexedDB is not available in this environment");
    }

    this.db = await new Promise((resolve, reject) => {
      const request = idbFactory.open(this.dbName, DB_VERSION);

      request.onupgradeneeded = (event) => {
        const db = event.target.result;

        // Spendable leaf pool. `data` is the full TreeNode; the other columns
        // are projected out of it so queries avoid deserializing the blob.
        if (!db.objectStoreNames.contains(STORE_LEAVES)) {
          const leaves = db.createObjectStore(STORE_LEAVES, { keyPath: "id" });
          leaves.createIndex("reservation_id", "reservation_id", {
            unique: false,
          });
          // Stored 0/1 rather than as a boolean: IndexedDB rejects booleans as
          // index keys, and indexing this is what keeps leavesMissingExitChains
          // off a full scan of the store.
          leaves.createIndex("chain_complete", "chain_complete", {
            unique: false,
          });
        }

        // Intermediate exit-chain nodes, kept separate from the leaf pool and
        // carrying no pool metadata (no reservation / missing / added_at). Each
        // row is owned by the leaf it belongs to (key `[leaf_id, id]`), so a
        // node shared by several leaves' chains stores one row per leaf rather
        // than one deduplicated row. Version 1 keyed this store by `id` alone;
        // a keyPath cannot be changed in place, and there is no data worth
        // migrating out of it, so it is dropped and recreated. Gated on
        // `oldVersion` so a later, unrelated version bump does not wipe it again.
        if (event.oldVersion < 2 && db.objectStoreNames.contains(STORE_ANCESTORS)) {
          db.deleteObjectStore(STORE_ANCESTORS);
        }
        if (!db.objectStoreNames.contains(STORE_ANCESTORS)) {
          const ancestors = db.createObjectStore(STORE_ANCESTORS, {
            keyPath: ["leaf_id", "id"],
          });
          ancestors.createIndex("leaf_id", "leaf_id", { unique: false });
        }

        // An existing wallet gets the flag derived from the ancestor rows it
        // already holds, so it does not refetch every chain. The index is
        // created here and populated by the rewrite below, since createIndex
        // cannot run from an async callback.
        if (event.oldVersion > 0 && event.oldVersion < DB_VERSION) {
          const tx = event.target.transaction;
          const leaves = tx.objectStore(STORE_LEAVES);
          if (!leaves.indexNames.contains("chain_complete")) {
            leaves.createIndex("chain_complete", "chain_complete", {
              unique: false,
            });
          }

          const ancestors = tx.objectStore(STORE_ANCESTORS);
          const linked = new Set();
          ancestors.openCursor().onsuccess = (cursorEvent) => {
            const cursor = cursorEvent.target.result;
            if (cursor) {
              linked.add(`${cursor.value.leaf_id}\u0000${cursor.value.id}`);
              cursor.continue();
              return;
            }
            leaves.openCursor().onsuccess = (leafEvent) => {
              const leafCursor = leafEvent.target.result;
              if (!leafCursor) return;
              const row = leafCursor.value;
              row.chain_complete =
                row.parent_node_id == null ||
                linked.has(`${row.id}\u0000${row.parent_node_id}`)
                  ? 1
                  : 0;
              leafCursor.update(row);
              leafCursor.continue();
            };
          };
        }

        if (!db.objectStoreNames.contains(STORE_RESERVATIONS)) {
          db.createObjectStore(STORE_RESERVATIONS, { keyPath: "id" });
        }

        if (!db.objectStoreNames.contains(STORE_SPENT)) {
          db.createObjectStore(STORE_SPENT, { keyPath: "id" });
        }

        if (!db.objectStoreNames.contains(STORE_SWAP_STATUS)) {
          db.createObjectStore(STORE_SWAP_STATUS, { keyPath: "id" });
        }
      };

      request.onsuccess = () => {
        const db = request.result;
        // Close on a version change requested by another connection so it is
        // not blocked by this one.
        db.onversionchange = () => {
          db.close();
          this.db = null;
        };
        resolve(db);
      };

      request.onerror = () =>
        reject(
          new TreeStoreError(
            `Failed to open IndexedDB: ${request.error?.message || "Unknown error"}`,
            request.error
          )
        );

      request.onblocked = () => {
        // Another open connection holds an older version. It will resolve once
        // that connection closes (see onversionchange above).
      };
    });

    return this;
  }

  close() {
    if (this.db) {
      this.db.close();
      this.db = null;
    }
  }

  // ===== Transaction runner =====

  /**
   * Runs one transaction. `reads` is a list of `{ name, store, key?, index?,
   * all? }`: with a key it is a `get`, without it a `getAll`, and `all` with a
   * key reads every record under that key; `index` reads through that
   * index on `store` instead of the store directly. All reads are issued up
   * front; once the last completes, `compute(results, tx)` runs synchronously
   * in that read's success handler and may issue writes on `tx`. Its return
   * value resolves the promise on `oncomplete`, so writes are durable before
   * resolve. `compute` must never await a non-IDB promise (it would close `tx`).
   */
  _txRun(storeNames, mode, reads, compute) {
    return new Promise((resolve, reject) => {
      if (!this.db) {
        reject(new TreeStoreError("Database not initialized"));
        return;
      }

      let tx;
      try {
        tx = this.db.transaction(storeNames, mode);
      } catch (e) {
        reject(e);
        return;
      }

      let outcome;
      let settled = false;
      let computeError = null;
      const done = (fn) => {
        if (!settled) {
          settled = true;
          fn();
        }
      };

      tx.oncomplete = () => done(() => resolve(outcome));
      tx.onabort = () =>
        done(() =>
          reject(
            computeError ||
              new TreeStoreError(
                `transaction aborted: ${tx.error?.message || "unknown"}`,
                tx.error
              )
          )
        );
      tx.onerror = () =>
        done(() =>
          reject(
            computeError ||
              new TreeStoreError(
                `transaction error: ${tx.error?.message || "unknown"}`,
                tx.error
              )
          )
        );

      const runCompute = (results) => {
        try {
          outcome = compute(results, tx);
        } catch (e) {
          computeError = e;
          try {
            tx.abort();
          } catch (_) {
            done(() => reject(e));
          }
        }
      };

      if (reads.length === 0) {
        runCompute({});
        return;
      }

      const results = {};
      let pending = reads.length;
      for (const r of reads) {
        let req;
        try {
          const store = tx.objectStore(r.store);
          const source = r.index ? store.index(r.index) : store;
          if (r.keysOnly) {
            // Primary keys of the matching records, without reading a value.
            req = source.getAllKeys(r.key);
          } else if (r.all) {
            // Every record under `key`, rather than the single first match a
            // bare `get` returns. An index key matches many records.
            req = source.getAll(r.key);
          } else {
            req = "key" in r ? source.get(r.key) : source.getAll();
          }
        } catch (e) {
          done(() => reject(e));
          return;
        }
        req.onsuccess = () => {
          results[r.name] = req.result;
          pending -= 1;
          if (pending === 0) runCompute(results);
        };
        // A failed request bubbles to the transaction and aborts it, surfacing
        // via tx.onabort above.
      }
    });
  }

  // ===== Reads =====

  /**
   * Reconstruct the exit chains for many leaves in one transaction, each as
   * { leaf, ancestors } with ancestors nearest first. A leaf absent from the store
   * is skipped; a chain that hits a gap comes back partial.
   * @param {Array<string>} leafIds
   * @returns {Promise<Array<{leaf: object, ancestors: Array<object>}>>}
   */
  async getExitChains(leafIds) {
    try {
      if (!leafIds || leafIds.length === 0) return [];
      return await new Promise((resolve, reject) => {
        if (!this.db) {
          reject(new TreeStoreError("Database not initialized"));
          return;
        }
        // Each requested leaf's own ancestor rows (via the leaf_id index) plus
        // the leaf itself, walked one chain at a time so a node id stored under
        // another leaf can never cross-contaminate this one. All reads are
        // issued up front; oncomplete only fires once every one of them (and
        // any request queued afterwards) has settled.
        const tx = this.db.transaction([STORE_LEAVES, STORE_ANCESTORS], "readonly");
        const leavesStore = tx.objectStore(STORE_LEAVES);
        const ancestorsIndex = tx.objectStore(STORE_ANCESTORS).index("leaf_id");

        const leafById = new Map();
        const ancestorsByLeaf = new Map();
        let settled = false;
        const fail = (msg) => {
          if (!settled) {
            settled = true;
            reject(new TreeStoreError(msg));
          }
        };
        tx.onabort = () =>
          fail(`Failed to get exit chains: ${tx.error?.message || "aborted"}`);
        tx.onerror = () =>
          fail(`Failed to get exit chains: ${tx.error?.message || "error"}`);
        tx.oncomplete = () => {
          if (settled) return;
          settled = true;
          const result = [];
          for (const id of leafIds) {
            const leafRow = leafById.get(id);
            if (!leafRow) continue;
            const nodes = ancestorsByLeaf.get(id) || new Map();
            nodes.set(leafRow.id, leafRow.data);
            const pedigree = assembleExitChain(nodes, id);
            if (pedigree) result.push(pedigree);
          }
          resolve(result);
        };

        for (const id of leafIds) {
          const leafReq = leavesStore.get(id);
          leafReq.onsuccess = () => {
            if (leafReq.result) leafById.set(id, leafReq.result);
          };
          const ancestorsReq = ancestorsIndex.getAll(id);
          ancestorsReq.onsuccess = () => {
            const nodes = new Map();
            for (const row of ancestorsReq.result || []) nodes.set(row.id, row.data);
            ancestorsByLeaf.set(id, nodes);
          };
        }
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to get exit chains: ${error.message}`, error);
    }
  }

  /**
   * Ids of the stored leaves whose chain cannot back an exit: the leaf has a
   * parent, and no ancestor row of its own holds that parent.
   * @returns {Promise<Array<string>>}
   */
  async leavesMissingExitChains() {
    try {
      // A leaf that is itself a root is always flagged complete, so the flag
      // alone selects what needs a chain, and the index yields the leaf ids
      // without deserializing a single stored node.
      return await this._txRun(
        [STORE_LEAVES],
        "readonly",
        [
          {
            name: "ids",
            store: STORE_LEAVES,
            index: "chain_complete",
            key: 0,
            keysOnly: true,
          },
        ],
        (res) => res.ids
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to get leaves missing exit chains: ${error.message}`,
        error
      );
    }
  }

  async getDeletedLeaves() {
    try {
      return await this._txRun(
        [STORE_LEAVES],
        "readonly",
        [{ name: "leaves", store: STORE_LEAVES }],
        (res) => res.leaves.filter((row) => row.is_deleted).map((row) => row.data)
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to get deleted leaves: ${error.message}`);
    }
  }

  async removeLeaves(leafIds) {
    try {
      if (!leafIds || leafIds.length === 0) return;
      const ids = new Set(leafIds);
      await this._txRun(
        [STORE_LEAVES, STORE_ANCESTORS],
        "readwrite",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "ancestors", store: STORE_ANCESTORS },
        ],
        (res, tx) => {
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const ancestorsStore = tx.objectStore(STORE_ANCESTORS);
          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));
          // Each leaf owns its chain, so its ancestor rows go with it. They are
          // keyed by [leaf_id, id], so the leaf's own id selects exactly its own.
          // Only a row still marked and still unreserved goes: the purge read
          // its list, then spent seconds asking the operators, and a refresh
          // landing in that window may have brought the leaf back or a payment
          // reserved it.
          for (const id of ids) {
            const row = leafMap.get(id);
            if (!row || !row.is_deleted || row.reservation_id != null) continue;
            leavesStore.delete(id);
            leafMap.delete(id);
            for (const anc of res.ancestors) {
              if (anc.leaf_id === id) ancestorsStore.delete([anc.leaf_id, anc.id]);
            }
          }
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to remove leaves: ${error.message}`);
    }
  }

  async getLeaves() {
    try {
      return await this._txRun(
        [STORE_LEAVES, STORE_RESERVATIONS],
        "readonly",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "reservations", store: STORE_RESERVATIONS },
        ],
        (res) => {
          const resMap = new Map(res.reservations.map((r) => [r.id, r]));
          const available = [];
          const notAvailable = [];
          const availableMissingFromOperators = [];
          const reservedForPayment = [];
          const reservedForSwap = [];

          for (const row of res.leaves) {
            const node = row.data;
            const purpose =
              row.reservation_id != null
                ? resMap.get(row.reservation_id)?.purpose
                : undefined;

            if (row.is_deleted) continue;
            const spendable = node.status === "Available";
            if (purpose) {
              if (purpose === "Payment") reservedForPayment.push(node);
              else if (purpose === "Swap") reservedForSwap.push(node);
            } else if (!spendable) {
              notAvailable.push(node);
            } else if (row.is_missing_from_operators) {
              availableMissingFromOperators.push(node);
            } else {
              available.push(node);
            }
          }

          return {
            available,
            notAvailable,
            availableMissingFromOperators,
            reservedForPayment,
            reservedForSwap,
          };
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to get leaves: ${error.message}`, error);
    }
  }

  async getAvailableBalance() {
    try {
      return await this._txRun(
        [STORE_LEAVES, STORE_RESERVATIONS],
        "readonly",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "reservations", store: STORE_RESERVATIONS },
        ],
        (res) => {
          const resMap = new Map(res.reservations.map((r) => [r.id, r]));
          // Spendable = unreserved-available + swap-reserved (mirrors
          // Leaves::balance, which also counts missing-from-operators leaves
          // that are still Available and unreserved).
          let balance = 0n;
          for (const row of res.leaves) {
            const purpose =
              row.reservation_id != null
                ? resMap.get(row.reservation_id)?.purpose
                : undefined;
            const included =
              !row.is_deleted &&
              ((row.reservation_id == null && row.status === "Available") ||
                purpose === "Swap");
            if (included) balance += BigInt(row.value);
          }
          return balance;
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to get available balance: ${error.message}`,
        error
      );
    }
  }

  async getVerifiedLeafKeys() {
    try {
      return await this._txRun(
        [STORE_LEAVES, STORE_RESERVATIONS],
        "readonly",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "reservations", store: STORE_RESERVATIONS },
        ],
        (res) => {
          const resIds = new Set(res.reservations.map((r) => r.id));
          const out = [];
          for (const row of res.leaves) {
            const hasReservation =
              row.reservation_id != null && resIds.has(row.reservation_id);
            // Every reserved leaf plus every Available one; nothing that is
            // neither reserved nor Available.
            if (!row.is_deleted && (hasReservation || row.status === "Available")) {
              out.push([
                row.id,
                row.verifying_public_key,
                row.signing_public_key,
              ]);
            }
          }
          return out;
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to get verified leaf keys: ${error.message}`,
        error
      );
    }
  }

  async now() {
    return this._nowMs();
  }

  // ===== Writes =====

  async addLeaves(leaves) {
    try {
      if (!leaves || leaves.length === 0) return;
      const leafIds = leaves.map((l) => l.id);

      await this._txRun(
        [STORE_LEAVES, STORE_SPENT],
        "readwrite",
        [{ name: "leaves", store: STORE_LEAVES }],
        (res, tx) => {
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const spentStore = tx.objectStore(STORE_SPENT);

          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));

          // Re-adding a leaf clears any stale spent marker for it.
          for (const id of leafIds) spentStore.delete(id);

          for (const leaf of leaves) {
            leavesStore.put(this._leafRow(leaf, false, leafMap.get(leaf.id)));
          }
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to add leaves: ${error.message}`, error);
    }
  }

  async storeAncestors(pedigrees) {
    try {
      if (!pedigrees || pedigrees.length === 0) return;

      // Scoped to the pedigrees' own rows: reading the two stores whole would
      // deserialize every leaf and ancestor row the wallet holds to write a
      // handful of chains.
      const reads = [];
      for (const p of pedigrees) {
        reads.push({
          name: `leaf:${p.leaf.id}`,
          store: STORE_LEAVES,
          key: p.leaf.id,
        });
        reads.push({
          name: `ancestors:${p.leaf.id}`,
          store: STORE_ANCESTORS,
          index: "leaf_id",
          key: p.leaf.id,
          all: true,
        });
      }

      await this._txRun(
        [STORE_LEAVES, STORE_ANCESTORS],
        "readwrite",
        reads,
        (res, tx) => {
          const ancestorsStore = tx.objectStore(STORE_ANCESTORS);
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const ancestorRowsByLeaf = this._ancestorRowsByLeaf(
            pedigrees.flatMap((p) => res[`ancestors:${p.leaf.id}`] || [])
          );
          // A leaf can be spent between its chain being resolved and this write,
          // and a chain is only ever removed with its leaf. Writing one for a leaf
          // that is already gone would leave it behind for good.
          const leafRowsById = new Map();
          for (const p of pedigrees) {
            const row = res[`leaf:${p.leaf.id}`];
            if (row) {
              leafRowsById.set(row.id, row);
            }
          }

          for (const p of pedigrees) {
            const leafRow = leafRowsById.get(p.leaf.id);
            if (!leafRow) {
              continue;
            }
            this._replaceAncestors(
              ancestorsStore,
              ancestorRowsByLeaf,
              p.leaf.id,
              p.ancestors
            );
            // This write only touches ancestor rows, so the leaf's completeness
            // flag has to be refreshed alongside them. Judged against the stored
            // leaf, not the fetched one: a renewal may have reparented it while
            // the chain was in flight, in which case the chain that just arrived
            // no longer reaches it.
            const chainComplete = this._chainComplete(
              leafRow.data,
              p.ancestors,
              leafRow
            );
            if (chainComplete !== !!leafRow.chain_complete) {
              leavesStore.put({
                ...leafRow,
                chain_complete: chainComplete ? 1 : 0,
              });
            }
          }
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to store ancestors: ${error.message}`, error);
    }
  }

  async setLeaves(leaves, missingLeaves, refreshStartedAtMs) {
    try {
      const reported = leaves || [];
      const missing = missingLeaves || [];
      const refreshMs = refreshStartedAtMs;

      await this._txRun(
        [STORE_LEAVES, STORE_ANCESTORS, STORE_RESERVATIONS, STORE_SPENT, STORE_SWAP_STATUS],
        "readwrite",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "ancestors", store: STORE_ANCESTORS },
          { name: "reservations", store: STORE_RESERVATIONS },
          { name: "spent", store: STORE_SPENT },
          { name: "swap", store: STORE_SWAP_STATUS, key: SWAP_STATUS_ID },
        ],
        (res, tx) => {
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const ancestorsStore = tx.objectStore(STORE_ANCESTORS);
          const reservationsStore = tx.objectStore(STORE_RESERVATIONS);
          const spentStore = tx.objectStore(STORE_SPENT);

          const nowMs = this._nowMs();
          const leafMap = new Map(res.leaves.map((r) => [r.id, { ...r }]));
          const ancestorRowsByLeaf = this._ancestorRowsByLeaf(res.ancestors);

          // Release + drop stale reservations BEFORE the swap guard, otherwise a
          // stale Swap reservation would pin has_active_swap true forever and
          // set_leaves could never make progress.
          const staleCutoff = nowMs - RESERVATION_TIMEOUT_MS;
          const staleIds = new Set(
            res.reservations.filter((r) => r.created_at < staleCutoff).map((r) => r.id)
          );
          for (const id of staleIds) reservationsStore.delete(id);
          for (const row of leafMap.values()) {
            if (row.reservation_id != null && staleIds.has(row.reservation_id)) {
              row.reservation_id = null;
              leavesStore.put(row);
            }
          }
          const remainingReservations = res.reservations.filter(
            (r) => !staleIds.has(r.id)
          );

          // Swap guard: skip the refresh body if a swap is in flight or one
          // completed during the refresh (its change would otherwise be lost).
          const hasActiveSwap = remainingReservations.some((r) => r.purpose === "Swap");
          const swapCompletedDuringRefresh =
            !!res.swap && res.swap.last_completed_at >= refreshMs;
          if (hasActiveSwap || swapCompletedDuringRefresh) {
            return; // stale-reservation cleanup above still commits
          }

          // Prune old spent markers, then collect the ones still fresh enough to
          // suppress re-adding a just-spent leaf during this refresh window.
          const spentCleanupCutoff = refreshMs - SPENT_MARKER_CLEANUP_THRESHOLD_MS;
          for (const s of res.spent) {
            if (s.spent_at < spentCleanupCutoff) spentStore.delete(s.id);
          }
          const spentIds = new Set(
            res.spent.filter((s) => s.spent_at >= refreshMs).map((s) => s.id)
          );

          // Taken before the marking below, because a leaf reported again is
          // rebuilt from its pedigree and a refresh carries no chains. An empty
          // chain means "unknown", so the flag the row already held has to come
          // from here, or every refresh would report every leaf as missing one.
          const priorRows = new Map(leafMap);

          // Mark, rather than remove, the non-reserved leaves added before the
          // refresh started. A leaf no operator reports may still be ours, and
          // its stored transactions are the only way to exit it, so the row
          // stays, and its ancestor rows stay with it; the puts below clear the
          // mark on whatever came back. A leaf we spent ourselves is the one
          // absence already accounted for, so it goes for good and its ancestor
          // rows are dropped alongside it.
          const deletedIds = [];
          for (const row of Array.from(leafMap.values())) {
            if (row.reservation_id != null || row.added_at >= refreshMs) continue;
            if (spentIds.has(row.id)) {
              leavesStore.delete(row.id);
              leafMap.delete(row.id);
              deletedIds.push(row.id);
              continue;
            }
            if (!row.is_deleted) {
              const marked = { ...row, is_deleted: true };
              leavesStore.put(marked);
              leafMap.set(marked.id, marked);
            }
          }

          const liveLeaves = reported.filter((p) => !spentIds.has(p.id));
          const liveMissing = missing.filter((p) => !spentIds.has(p.id));

          for (const p of liveLeaves) {
            const row = this._leafRow(p, false, priorRows.get(p.id));
            leavesStore.put(row);
            leafMap.set(row.id, row);
          }
          for (const p of liveMissing) {
            const row = this._leafRow(p, true, priorRows.get(p.id));
            leavesStore.put(row);
            leafMap.set(row.id, row);
          }

          const survivingIds = new Set();
          for (const p of reported.concat(missing)) {
            if (!spentIds.has(p.id)) survivingIds.add(p.id);
          }
          for (const id of deletedIds) {
            if (survivingIds.has(id)) continue;
            for (const existingId of (ancestorRowsByLeaf.get(id) || new Map()).keys()) {
              ancestorsStore.delete([id, existingId]);
            }
          }
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to set leaves: ${error.message}`, error);
    }
  }

  async cancelReservation(id, leavesToKeep) {
    try {
      const keep = leavesToKeep || [];
      await this._txRun(
        [STORE_LEAVES, STORE_ANCESTORS, STORE_RESERVATIONS],
        "readwrite",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "ancestors", store: STORE_ANCESTORS },
          { name: "res", store: STORE_RESERVATIONS, key: id },
        ],
        (res, tx) => {
          // Return leavesToKeep to the pool even when the reservation is already
          // gone (e.g. released by stale cleanup): dropping them here would lose
          // the leaves until the next refresh. The deletes below no-op in that case.
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const ancestorsStore = tx.objectStore(STORE_ANCESTORS);
          const reservationsStore = tx.objectStore(STORE_RESERVATIONS);

          const keepIds = new Set(keep.map((l) => l.id));
          const ancestorRowsByLeaf = this._ancestorRowsByLeaf(res.ancestors);
          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));
          // A kept leaf keeps its ancestor rows, so the row rebuilt below has to
          // keep the flag that describes them.
          const priorRows = new Map(leafMap);
          for (const l of res.leaves) {
            if (l.reservation_id !== id) continue;
            leavesStore.delete(l.id);
            leafMap.delete(l.id);
            // A kept leaf's ancestor rows stay put; a dropped leaf's are
            // removed with it.
            if (!keepIds.has(l.id)) {
              for (const existingId of (ancestorRowsByLeaf.get(l.id) || new Map()).keys()) {
                ancestorsStore.delete([l.id, existingId]);
              }
            }
          }
          reservationsStore.delete(id);

          if (keep.length > 0) {
            // Only re-insert the leaves: a kept leaf's ancestor rows were left
            // untouched above.
            for (const leaf of keep) {
              // The chain flag carries over: those rows were deliberately kept.
              // So does any reservation other than this one, since a leaf the
              // refresh released and another reservation then took is not this
              // cancellation's to free.
              const prior = priorRows.get(leaf.id);
              const held = prior?.reservation_id === id ? undefined : prior;
              const chainComplete = this._chainComplete(leaf, undefined, prior);
              leavesStore.put(this._leafRow(leaf, false, held, chainComplete));
            }
          }
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to cancel reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  async finalizeReservation(id, newLeaves) {
    try {
      const added = newLeaves || null;
      await this._txRun(
        [STORE_LEAVES, STORE_ANCESTORS, STORE_RESERVATIONS, STORE_SPENT, STORE_SWAP_STATUS],
        "readwrite",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "ancestors", store: STORE_ANCESTORS },
          { name: "res", store: STORE_RESERVATIONS, key: id },
        ],
        (res, tx) => {
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const ancestorsStore = tx.objectStore(STORE_ANCESTORS);
          const reservationsStore = tx.objectStore(STORE_RESERVATIONS);
          const spentStore = tx.objectStore(STORE_SPENT);
          const swapStore = tx.objectStore(STORE_SWAP_STATUS);

          const nowMs = this._nowMs();
          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));
          const ancestorRowsByLeaf = this._ancestorRowsByLeaf(res.ancestors);

          let isSwap = false;
          if (res.res) {
            isSwap = res.res.purpose === "Swap";
            for (const l of res.leaves) {
              if (l.reservation_id === id) {
                spentStore.put({ id: l.id, spent_at: nowMs });
                leavesStore.delete(l.id);
                leafMap.delete(l.id);
                // The spent leaf owns these ancestor rows; remove them now
                // since there is no later reclaim pass.
                for (const existingId of (ancestorRowsByLeaf.get(l.id) || new Map()).keys()) {
                  ancestorsStore.delete([l.id, existingId]);
                }
              }
            }
            reservationsStore.delete(id);
          }

          if (added && added.length > 0) {
            for (const p of added) {
              const row = this._leafRow(p, false, leafMap.get(p.id));
              leavesStore.put(row);
              leafMap.set(row.id, row);
            }
          }

          if (isSwap && added && added.length > 0) {
            swapStore.put({ id: SWAP_STATUS_ID, last_completed_at: nowMs });
          }
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to finalize reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  async updateReservation(reservationId, reservedLeaves, changeLeaves) {
    try {
      const reserved = reservedLeaves || [];
      const change = changeLeaves || [];
      return await this._txRun(
        [STORE_LEAVES, STORE_ANCESTORS, STORE_RESERVATIONS, STORE_SPENT],
        "readwrite",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "ancestors", store: STORE_ANCESTORS },
          { name: "res", store: STORE_RESERVATIONS, key: reservationId },
        ],
        (res, tx) => {
          if (!res.res) {
            throw new TreeStoreError(`Reservation ${reservationId} not found`);
          }
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const ancestorsStore = tx.objectStore(STORE_ANCESTORS);
          const reservationsStore = tx.objectStore(STORE_RESERVATIONS);
          const spentStore = tx.objectStore(STORE_SPENT);

          const nowMs = this._nowMs();
          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));
          const ancestorRowsByLeaf = this._ancestorRowsByLeaf(res.ancestors);

          // Old reserved leaves are consumed by the swap: mark spent and drop,
          // along with the ancestor rows they own, since there is no later
          // reclaim pass.
          for (const l of res.leaves) {
            if (l.reservation_id === reservationId) {
              spentStore.put({ id: l.id, spent_at: nowMs });
              leavesStore.delete(l.id);
              leafMap.delete(l.id);
              for (const existingId of (ancestorRowsByLeaf.get(l.id) || new Map()).keys()) {
                ancestorsStore.delete([l.id, existingId]);
              }
            }
          }


          const leafNodes = change.concat(reserved);

          // Change leaves go back to the available pool.
          for (const p of change) {
            const row = this._leafRow(p, false, leafMap.get(p.id));
            leavesStore.put(row);
            leafMap.set(row.id, row);
          }
          // Reserved leaves stay attached to this same reservation.
          for (const p of reserved) {
            const row = this._leafRow(p, false, leafMap.get(p.id));
            row.reservation_id = reservationId;
            leavesStore.put(row);
            leafMap.set(row.id, row);
          }

          reservationsStore.put({ ...res.res, pending_change_amount: 0 });

          // Return value must be plain TreeNodes: the Rust side deserializes
          // Vec<TreeNode>.
          return { id: reservationId, leaves: reserved };
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to update reservation '${reservationId}': ${error.message}`,
        error
      );
    }
  }

  async tryReserveLeaves(targetAmounts, exactOnly, purpose) {
    try {
      return await this._txRun(
        [STORE_LEAVES, STORE_RESERVATIONS],
        "readwrite",
        [
          { name: "leaves", store: STORE_LEAVES },
          { name: "reservations", store: STORE_RESERVATIONS },
        ],
        (res, tx) => {
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const reservationsStore = tx.objectStore(STORE_RESERVATIONS);

          const targetAmount = targetAmounts ? this._totalSats(targetAmounts) : 0;
          const maxTarget = this._maxTargetForPrefilter(targetAmounts);

          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));
          const eligible = this._eligibleSlim(res.leaves);
          // True total over ALL eligible leaves, not the prefiltered set: the
          // WaitForPending decision below must not be derived from the slim set.
          const available = eligible.reduce((s, l) => s + l.value, 0);
          const slim = this._slimCandidates(eligible, maxTarget);
          const pending = res.reservations.reduce(
            (s, r) => s + (r.pending_change_amount || 0),
            0
          );

          const selected = this._selectLeavesByTargetAmounts(slim, targetAmounts);
          if (selected !== null) {
            if (selected.length === 0) {
              throw new TreeStoreError("NonReservableLeaves");
            }
            const ids = selected.map((s) => s.id);
            const fullLeaves = ids.map((leafId) => leafMap.get(leafId).data);
            const reservationId = this._generateId();
            this._createReservation(
              reservationsStore,
              leavesStore,
              leafMap,
              reservationId,
              ids,
              purpose,
              0
            );
            return {
              type: "success",
              reservation: { id: reservationId, leaves: fullLeaves },
            };
          }

          if (!exactOnly) {
            const minSelected = this._selectLeavesByMinimumAmount(slim, targetAmount);
            if (minSelected !== null) {
              const ids = minSelected.map((s) => s.id);
              const fullLeaves = ids.map((leafId) => leafMap.get(leafId).data);
              const reservedAmount = fullLeaves.reduce((s, l) => s + l.value, 0);
              const pendingChange =
                reservedAmount > targetAmount && targetAmount > 0
                  ? reservedAmount - targetAmount
                  : 0;
              const reservationId = this._generateId();
              this._createReservation(
                reservationsStore,
                leavesStore,
                leafMap,
                reservationId,
                ids,
                purpose,
                pendingChange
              );
              return {
                type: "success",
                reservation: { id: reservationId, leaves: fullLeaves },
              };
            }
          }

          if (available + pending >= targetAmount) {
            return {
              type: "waitForPending",
              needed: targetAmount,
              available,
              pending,
            };
          }
          return { type: "insufficientFunds" };
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try reserve leaves: ${error.message}`,
        error
      );
    }
  }

  async tryReserveLeavesByIds(leafIds, purpose) {
    try {
      if (!leafIds || leafIds.length === 0) {
        throw new TreeStoreError("NonReservableLeaves");
      }
      return await this._txRun(
        [STORE_LEAVES, STORE_RESERVATIONS],
        "readwrite",
        [{ name: "leaves", store: STORE_LEAVES }],
        (res, tx) => {
          const leavesStore = tx.objectStore(STORE_LEAVES);
          const reservationsStore = tx.objectStore(STORE_RESERVATIONS);

          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));
          const eligibleIds = new Set(
            res.leaves
              .filter(
                (r) =>
                  r.status === "Available" &&
                  !r.is_missing_from_operators &&
                  !r.is_deleted &&
                  r.reservation_id == null
              )
              .map((r) => r.id)
          );
          // Every requested leaf must be available and unreserved (and the ids
          // distinct); otherwise reserve nothing.
          const matched = new Set(leafIds.filter((id) => eligibleIds.has(id)));
          if (matched.size !== leafIds.length) {
            throw new TreeStoreError("NonReservableLeaves");
          }

          const fullLeaves = leafIds.map((id) => leafMap.get(id).data);
          const reservationId = this._generateId();
          this._createReservation(
            reservationsStore,
            leavesStore,
            leafMap,
            reservationId,
            leafIds,
            purpose,
            0
          );
          return { id: reservationId, leaves: fullLeaves };
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try reserve leaves by ids: ${error.message}`,
        error
      );
    }
  }

  async trySelectLeaves(targetAmounts) {
    try {
      const targetAmount = targetAmounts ? this._totalSats(targetAmounts) : 0;
      const maxTarget = this._maxTargetForPrefilter(targetAmounts);
      return await this._txRun(
        [STORE_LEAVES],
        "readonly",
        [{ name: "leaves", store: STORE_LEAVES }],
        (res) => {
          const leafMap = new Map(res.leaves.map((r) => [r.id, r]));
          const slim = this._slimCandidates(this._eligibleSlim(res.leaves), maxTarget);

          const selected = this._selectLeavesByTargetAmounts(slim, targetAmounts);
          if (selected !== null && selected.length > 0) {
            return {
              type: "exact",
              leaves: selected.map((s) => leafMap.get(s.id).data),
            };
          }

          const minSelected = this._selectLeavesByMinimumAmount(slim, targetAmount);
          if (minSelected !== null) {
            return {
              type: "swapNeeded",
              leaves: minSelected.map((s) => leafMap.get(s.id).data),
            };
          }

          return { type: "insufficientFunds" };
        }
      );
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try select leaves: ${error.message}`,
        error
      );
    }
  }

  // ===== Private helpers =====

  _nowMs() {
    return Date.now();
  }

  _generateId() {
    if (typeof crypto !== "undefined" && crypto.randomUUID) {
      return crypto.randomUUID();
    }
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
      const r = (Math.random() * 16) | 0;
      const v = c === "x" ? r : (r & 0x3) | 0x8;
      return v.toString(16);
    });
  }

  /** Row shape for the leaf pool. Preserves `reservation_id` from an existing
   *  row; every other field comes from the operators' latest copy. */
  _leafRow(node, isMissing, existingRow, chainComplete) {
    return {
      id: node.id,
      parent_node_id: node.parent_node_id ?? null,
      status: node.status,
      value: node.value,
      verifying_public_key: node.verifying_public_key,
      signing_public_key: node.signing_keyshare.public_key,
      is_missing_from_operators: !!isMissing,
      is_deleted: false,
      reservation_id: existingRow ? existingRow.reservation_id ?? null : null,
      added_at: this._nowMs(),
      // Denormalized and indexed so leavesMissingExitChains reads only the ids
      // it wants, rather than every leaf row. Stored 0/1: IndexedDB rejects
      // booleans as index keys.
      chain_complete: (
        chainComplete !== undefined
          ? chainComplete
          : this._chainComplete(node, undefined, existingRow)
      )
        ? 1
        : 0,
      data: node,
    };
  }

  /**
   * Whether `node` has a stored chain reaching its current parent, which is what
   * makes it exitable without the operators. A stored chain runs from some parent
   * to a root, so holding the parent the leaf has now means it spans the whole
   * path. A leaf that is itself a root needs no ancestors. An empty incoming
   * chain means "unknown", not "none", so it keeps whatever the row already
   * claimed, unless the leaf has been reparented since: the chain then describes
   * the parent it had before.
   */
  _chainComplete(node, ancestors, existingRow) {
    if (node.parent_node_id == null) return true;
    if (ancestors && ancestors.length > 0) {
      return ancestors.some((a) => a.id === node.parent_node_id);
    }
    if (!existingRow) return false;
    return (
      existingRow.parent_node_id === node.parent_node_id &&
      !!existingRow.chain_complete
    );
  }

  /** Row shape for an ancestor: no pool metadata (reservation / missing / added_at).
   *  `leaf_id` plus `id` is the key, so the same node stores one row per leaf
   *  that descends from it rather than one row shared by all of them. */
  _ancestorRow(leafId, node) {
    return {
      leaf_id: leafId,
      id: node.id,
      parent_node_id: node.parent_node_id ?? null,
      status: node.status,
      value: node.value,
      verifying_public_key: node.verifying_public_key,
      signing_public_key: node.signing_keyshare.public_key,
      data: node,
    };
  }

  /** Groups ancestor rows (from a full-store read) by the leaf that owns them,
   *  as `Map<leaf_id, Map<id, row>>`, so a leaf's existing rows can be found
   *  and deleted by key without a range scan on the compound `[leaf_id, id]`
   *  keyPath. */
  _ancestorRowsByLeaf(ancestorRows) {
    const map = new Map();
    for (const row of ancestorRows || []) {
      let byId = map.get(row.leaf_id);
      if (!byId) {
        byId = new Map();
        map.set(row.leaf_id, byId);
      }
      byId.set(row.id, row);
    }
    return map;
  }

  /**
   * Replaces `leafId`'s ancestor rows wholesale (delete then insert), keeping
   * `ancestorRowsByLeaf` current so a later step in the same transaction sees
   * this leaf's live rows rather than a stale snapshot. An empty `nodes` list
   * is a no-op: it means the chain is unknown, not that the leaf has none.
   */
  _replaceAncestors(ancestorsStore, ancestorRowsByLeaf, leafId, nodes) {
    if (!nodes || nodes.length === 0) return;

    const ownRows = ancestorRowsByLeaf.get(leafId);
    if (ownRows) {
      for (const existingId of ownRows.keys()) {
        ancestorsStore.delete([leafId, existingId]);
      }
    }
    const newRows = new Map();
    for (const node of nodes) {
      const row = this._ancestorRow(leafId, node);
      ancestorsStore.put(row);
      newRows.set(node.id, row);
    }
    ancestorRowsByLeaf.set(leafId, newRows);
  }

  _createReservation(
    reservationsStore,
    leavesStore,
    leafMap,
    reservationId,
    leafIds,
    purpose,
    pendingChange
  ) {
    reservationsStore.put({
      id: reservationId,
      purpose,
      pending_change_amount: pendingChange,
      created_at: this._nowMs(),
    });
    for (const id of leafIds) {
      const row = leafMap.get(id);
      if (row) {
        row.reservation_id = reservationId;
        leavesStore.put(row);
      }
    }
  }

  /** Slim `{id, value}` projection of the leaves eligible for selection. */
  _eligibleSlim(leafRows) {
    return leafRows
      .filter(
        (r) =>
          r.status === "Available" &&
          !r.is_missing_from_operators &&
          !r.is_deleted &&
          r.reservation_id == null
      )
      .map((r) => ({ id: r.id, value: r.value }));
  }

  /**
   * Prefilter mirroring the SQL slim candidate set: every eligible leaf with
   * value <= maxTarget, plus the single smallest leaf with value > maxTarget
   * (the minimum-amount fallback where one larger leaf alone suffices).
   */
  _slimCandidates(eligible, maxTarget) {
    const small = eligible.filter((l) => l.value <= maxTarget);
    let smallestBig = null;
    for (const l of eligible) {
      if (l.value > maxTarget && (smallestBig === null || l.value < smallestBig.value)) {
        smallestBig = l;
      }
    }
    return smallestBig ? [...small, smallestBig] : small;
  }

  _maxTargetForPrefilter(targetAmounts) {
    if (!targetAmounts) return Number.MAX_SAFE_INTEGER;
    if (targetAmounts.type === "amountAndFee") {
      return targetAmounts.amountSats + (targetAmounts.feeSats || 0);
    }
    if (targetAmounts.type === "exactDenominations") {
      return targetAmounts.denominations.reduce((m, v) => m + v, 0);
    }
    return Number.MAX_SAFE_INTEGER;
  }

  _totalSats(targetAmounts) {
    if (targetAmounts.type === "amountAndFee") {
      return targetAmounts.amountSats + (targetAmounts.feeSats || 0);
    }
    if (targetAmounts.type === "exactDenominations") {
      return targetAmounts.denominations.reduce((sum, d) => sum + d, 0);
    }
    return 0;
  }

  _selectLeavesByTargetAmounts(leaves, targetAmounts) {
    if (!targetAmounts) {
      return [...leaves];
    }

    if (targetAmounts.type === "amountAndFee") {
      const amountLeaves = this._selectLeavesByExactAmount(
        leaves,
        targetAmounts.amountSats
      );
      if (amountLeaves === null) return null;

      if (targetAmounts.feeSats != null && targetAmounts.feeSats > 0) {
        const amountIds = new Set(amountLeaves.map((l) => l.id));
        const remaining = leaves.filter((l) => !amountIds.has(l.id));
        const feeLeaves = this._selectLeavesByExactAmount(
          remaining,
          targetAmounts.feeSats
        );
        if (feeLeaves === null) return null;
        return [...amountLeaves, ...feeLeaves];
      }

      return amountLeaves;
    }

    if (targetAmounts.type === "exactDenominations") {
      return this._selectLeavesByExactDenominations(
        leaves,
        targetAmounts.denominations
      );
    }

    return null;
  }

  _selectLeavesByExactAmount(leaves, targetAmount) {
    if (targetAmount === 0) return null;

    const totalAvailable = leaves.reduce((sum, l) => sum + l.value, 0);
    if (totalAvailable < targetAmount) return null;

    const single = leaves.find((l) => l.value === targetAmount);
    if (single) return [single];

    const multipleResult = this._findExactMultipleMatch(leaves, targetAmount);
    return multipleResult;
  }

  _selectLeavesByExactDenominations(leaves, denominations) {
    const remaining = [...leaves];
    const selected = [];

    for (const denomination of denominations) {
      const idx = remaining.findIndex((l) => l.value === denomination);
      if (idx === -1) return null;
      selected.push(remaining[idx]);
      remaining.splice(idx, 1);
    }

    return selected;
  }

  _selectLeavesByMinimumAmount(leaves, targetAmount) {
    if (targetAmount === 0) return null;

    const totalAvailable = leaves.reduce((sum, l) => sum + l.value, 0);
    if (totalAvailable < targetAmount) return null;

    const result = [];
    let sum = 0;
    for (const leaf of leaves) {
      sum += leaf.value;
      result.push(leaf);
      if (sum >= targetAmount) break;
    }

    return sum >= targetAmount ? result : null;
  }

  _findExactMultipleMatch(leaves, targetAmount) {
    if (targetAmount === 0) return [];
    if (leaves.length === 0) return null;

    const result = this._greedyExactMatch(leaves, targetAmount);
    if (result) return result;

    const powerOfTwoLeaves = leaves.filter((l) => this._isPowerOfTwo(l.value));
    if (powerOfTwoLeaves.length === leaves.length) return null;

    return this._greedyExactMatch(powerOfTwoLeaves, targetAmount);
  }

  _greedyExactMatch(leaves, targetAmount) {
    const sorted = [...leaves].sort((a, b) => b.value - a.value);
    const result = [];
    let remaining = targetAmount;

    for (const leaf of sorted) {
      if (leaf.value > remaining) continue;
      remaining -= leaf.value;
      result.push(leaf);
      if (remaining === 0) return result;
    }

    return null;
  }

  _isPowerOfTwo(value) {
    return value > 0 && (value & (value - 1)) === 0;
  }
}

/**
 * Opens (or creates) the IndexedDB database and returns a ready tree store.
 *
 * @param {string} dbName - Database name (one database per SDK instance).
 * @param {object} [logger] - Optional logger.
 * @returns {Promise<WebTreeStore>}
 */
export async function createWebTreeStore(dbName, logger = null) {
  const store = new WebTreeStore(dbName, logger);
  await store.initialize();
  return store;
}

/**
 * Deletes a tree store database. Intended for tests that need a clean database;
 * production code never calls this.
 *
 * @param {string} dbName
 * @returns {Promise<void>}
 */
export async function deleteWebTreeStore(dbName) {
  await new Promise((resolve, reject) => {
    const req = indexedDB.deleteDatabase(dbName);
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
    // A stale open connection can block deletion; proceed anyway.
    req.onblocked = () => resolve();
  });
}

export { WebTreeStore, TreeStoreError };
