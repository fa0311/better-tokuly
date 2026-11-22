/**
 * Where the catalogue copy rests between visits: one record in IndexedDB.
 *
 * IndexedDB can fail in more ways than "not available": an `open` may never
 * answer (a database held by a tab running older code, a profile on a slow
 * or broken disk) or be blocked. The catalogue must not wait on any of that,
 * so every operation here gives up after a moment: a read then finds
 * nothing (the catalogue is fetched whole) and a write is dropped.
 */

/** The catalogue as it is stored: plain data, the version as a decimal string (older browsers cannot store BigInt). */
export interface StoredCatalog<Emoji = unknown, Category = unknown> {
  version: string;
  emojis: Emoji[];
  categories: Category[];
}

export interface CatalogStore<Emoji, Category> {
  /** The stored copy, or `undefined` when there is none or the database does not answer in time. */
  read(): Promise<StoredCatalog<Emoji, Category> | undefined>;
  /** Replaces the stored copy. Never rejects. */
  write(record: StoredCatalog<Emoji, Category>): Promise<void>;
}

const DB_NAME = "tokuly.emoji";
const STORE = "catalog";
const RECORD = "approved";
/** How long the database gets to open, and a read or write to finish. */
export const STORE_TIMEOUT_MS = 1500;

function withTimeout<T>(work: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("the emoji database did not answer")), ms);
    work.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

/**
 * A store on the given IndexedDB (the browser's by default; tests hand in
 * their own). One connection serves every read and write; it is closed when
 * another tab wants to upgrade the database, and opened again on next use.
 */
export function createCatalogStore<Emoji, Category>(factory: () => IDBFactory | undefined = () => (typeof indexedDB === "undefined" ? undefined : indexedDB), timeoutMs = STORE_TIMEOUT_MS): CatalogStore<Emoji, Category> {
  let connection: Promise<IDBDatabase> | null = null;

  const open = (): Promise<IDBDatabase> => {
    if (connection) return connection;
    const idb = factory();
    if (!idb) return Promise.reject(new Error("no IndexedDB"));
    let abandoned = false;
    const opening = new Promise<IDBDatabase>((resolve, reject) => {
      const request = idb.open(DB_NAME, 1);
      request.onupgradeneeded = () => request.result.createObjectStore(STORE);
      request.onsuccess = () => {
        const db = request.result;
        // Too late: nobody waits for this connection any more.
        if (abandoned) return db.close();
        const forget = () => {
          if (connection === attempt) connection = null;
        };
        // Another tab (newer code) wants to change the database: get out of its way.
        db.onversionchange = () => {
          db.close();
          forget();
        };
        db.onclose = forget;
        resolve(db);
      };
      request.onerror = () => reject(request.error ?? new Error("the emoji database could not be opened"));
      // Another connection holds an older version open; waiting for it could take the whole visit.
      request.onblocked = () => reject(new Error("the emoji database is blocked"));
    });
    const attempt = withTimeout(opening, timeoutMs);
    connection = attempt;
    // A failed or silent open is not remembered: the next use tries again.
    attempt.catch(() => {
      abandoned = true;
      if (connection === attempt) connection = null;
    });
    return attempt;
  };

  return {
    async read() {
      try {
        const db = await open();
        return await withTimeout(
          new Promise<StoredCatalog<Emoji, Category> | undefined>((resolve, reject) => {
            const request = db.transaction(STORE, "readonly").objectStore(STORE).get(RECORD);
            request.onsuccess = () => resolve(request.result as StoredCatalog<Emoji, Category> | undefined);
            request.onerror = () => reject(request.error);
          }),
          timeoutMs,
        );
      } catch {
        // Private mode, a blocked, broken or silent database: the catalogue is simply fetched again.
        return undefined;
      }
    },
    async write(record) {
      try {
        const db = await open();
        await withTimeout(
          new Promise<void>((resolve, reject) => {
            const transaction = db.transaction(STORE, "readwrite");
            transaction.objectStore(STORE).put(record, RECORD);
            transaction.oncomplete = () => resolve();
            transaction.onerror = () => reject(transaction.error);
            transaction.onabort = () => reject(transaction.error);
          }),
          timeoutMs,
        );
      } catch {
        // Nothing to keep it in: this visit works from memory.
      }
    },
  };
}
