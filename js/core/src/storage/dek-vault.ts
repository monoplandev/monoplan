// Persistent web session vault.
//
// DEKs only ever live in memory after login. Re-prompting for the
// password on every reload would require a fresh Argon2id run and is
// the wrong UX bar for this product, so we persist them at rest —
// but never as plaintext. Each DEK's bytes are AES-GCM encrypted under
// a per-vault wrapping key that's generated `extractable: false` and
// stored in IndexedDB. A non-extractable CryptoKey can be used by the
// origin's pages to encrypt/decrypt but cannot be read back out, so a
// passive snapshot/exfil of IDB yields ciphertext the attacker cannot
// open. An XSS payload running while the page is open *can* call
// `crypto.subtle.decrypt` with the live key — that's the documented
// tradeoff.
//
// One record per origin keyed at `'current'`. Web is a single-account
// surface; logging in as a different account overwrites. The record
// holds **one wrapped entry per doc id** the account can open (the
// primary / Home doc included), each under its own IV but the same
// wrapping key, so a reload restores every membership without a
// network round trip (sharing-plan.md Phase 5, decided 2026-10-20).
// `putDocKey` adds an entry for a doc that appears after login (one
// created on this device, or learned from `GET /api/docs`).
//
// The vault is one store in the `monoplan-web` IndexedDB database; the
// schema (every store) is owned by `./web-db.ts` so no module's open
// path surprises another with a missing object store.

import type { Dek } from "../../wasm/monoplan_core_web.js";
import { STORE_VAULT, openMonoplanDb } from "./web-db.ts";

const KEY = "current";

export interface VaultedSession {
  /** Local-only sessions with no server account behind them. Email
   *  and deviceId are null in this mode. */
  anonymous: boolean;
  accountId: string;
  /** Doc id of the account's primary (Home) doc. Server-assigned for
   *  authenticated sessions, locally generated for anonymous ones. */
  primaryDocId: string;
  email: string | null;
  deviceId: string | null;
  /** The Home DEK — the entry in `docKeys` for `primaryDocId`. */
  dek: Dek;
  /** Every doc the account can open, keyed by doc id, primary included.
   *  `load` returns fresh `Dek` handles; `save` reads (and never
   *  consumes) the ones passed in. */
  docKeys: Map<string, Dek>;
}

/** One AES-GCM-wrapped DEK. */
interface WrappedEntry {
  iv: Uint8Array;
  wrappedDek: Uint8Array;
}

export interface VaultRecord {
  anonymous: boolean;
  accountId: string;
  primaryDocId: string;
  email: string | null;
  deviceId: string | null;
  wrappingKey: CryptoKey;
  /** Keyed by doc id; always contains `primaryDocId`. */
  docKeys: Record<string, WrappedEntry>;
}

/** Static `Dek.fromHex` — injected so this module type-imports wasm
 *  only, keeping core's index.ts re-exports wasm-free at load time. */
export type DekFromHex = (hex: string) => Dek;

/**
 * Single-origin DEK vault. One instance per app lifetime; the wrapping
 * key and IDB record are tied to the origin, not to the live session,
 * so re-instantiation on logout/login is cheap and safe.
 */
export class DekVault {
  constructor(private readonly dekFromHex: DekFromHex) {}

  async load(): Promise<VaultedSession | null> {
    let rec: VaultRecord | undefined;
    try {
      const db = await openMonoplanDb();
      rec = await idbGet<VaultRecord>(db, STORE_VAULT, KEY);
    } catch (e) {
      console.warn("DekVault.load: idb open/read failed:", e);
      return null;
    }
    if (!rec) return null;
    try {
      const s = await openRecord(rec, this.dekFromHex);
      if (s === null) {
        // Record predates a field this build requires; the only way to
        // recover is to log in again so the next write captures it.
        // Surface as "no session" so callers prompt for auth.
        await this.clear();
      }
      return s;
    } catch (e) {
      // Corrupt or tampered record — drop it so the next login can
      // write a fresh one.
      console.warn("DekVault.load: unwrap failed, clearing:", e);
      await this.clear();
      return null;
    }
  }

  /** Replace the record wholesale: a fresh wrapping key, every entry in
   *  `s.docKeys` re-wrapped under it. */
  async save(s: VaultedSession): Promise<void> {
    const rec = await sealSession(s);
    const db = await openMonoplanDb();
    await idbPut(db, STORE_VAULT, KEY, rec);
  }

  /** Add (or replace) one doc's entry in the stored record, under the
   *  record's existing wrapping key. Wrapping is async (WebCrypto), and
   *  an IDB transaction auto-commits as soon as control returns to the
   *  event loop with no request pending, so this is read → wrap → merge
   *  rather than one transaction: the merge re-reads the record and
   *  only adds the entry, so another `putDocKey` landing in between is
   *  kept, and a `save` (logout / login) in between — a different
   *  account's record — is left alone. Throws if there is no record to
   *  add to: the session must have been saved first. */
  async putDocKey(docId: string, dek: Dek): Promise<void> {
    const db = await openMonoplanDb();
    const rec = await idbGet<VaultRecord>(db, STORE_VAULT, KEY);
    if (!rec) throw new Error("DekVault.putDocKey: no vaulted session");
    const entry = await wrapOne(rec.wrappingKey, dek);
    await idbUpdate<VaultRecord>(db, STORE_VAULT, KEY, (current) => {
      if (!current || current.accountId !== rec.accountId) return undefined;
      return { ...current, docKeys: { ...current.docKeys, [docId]: entry } };
    });
  }

  /** Drop one doc's entry from the stored record (a doc deleted, or no
   *  longer listed for the account). Same read → merge shape as
   *  `putDocKey`; a missing record or entry is a no-op. The primary
   *  doc's entry is never removed. */
  async removeDocKey(docId: string): Promise<void> {
    const db = await openMonoplanDb();
    await idbUpdate<VaultRecord>(db, STORE_VAULT, KEY, (current) => {
      if (!current || current.primaryDocId === docId || !(docId in current.docKeys)) {
        return undefined;
      }
      const docKeys = { ...current.docKeys };
      delete docKeys[docId];
      return { ...current, docKeys };
    });
  }

  async clear(): Promise<void> {
    try {
      const db = await openMonoplanDb();
      await idbDelete(db, STORE_VAULT, KEY);
    } catch (e) {
      console.warn("DekVault.clear: idb delete failed:", e);
    }
  }
}

// ---------- record <-> session (pure; IDB-free, tested directly) ----------

/** Build the at-rest record for `s`: fresh non-extractable wrapping key,
 *  one entry per doc id. Does not consume the session's `Dek`s. Throws if
 *  `docKeys` lacks the primary doc. */
export async function sealSession(s: VaultedSession): Promise<VaultRecord> {
  if (!s.docKeys.has(s.primaryDocId)) {
    throw new Error("sealSession: docKeys has no entry for the primary doc");
  }
  const wrappingKey = await crypto.subtle.generateKey(
    { name: "AES-GCM", length: 256 },
    false, // non-extractable: stays inside the browser's key store
    ["encrypt", "decrypt"],
  );
  const docKeys: Record<string, WrappedEntry> = {};
  for (const [docId, dek] of s.docKeys) {
    docKeys[docId] = await wrapOne(wrappingKey, dek);
  }
  return {
    anonymous: s.anonymous,
    accountId: s.accountId,
    primaryDocId: s.primaryDocId,
    email: s.email,
    deviceId: s.deviceId,
    wrappingKey,
    docKeys,
  };
}

/** Unwrap every entry of a stored record. Returns `null` for a record
 *  that lacks a field this build requires (caller clears it); throws if
 *  any entry fails to decrypt (tampered / wrong key). */
export async function openRecord(
  rec: VaultRecord,
  dekFromHex: DekFromHex,
): Promise<VaultedSession | null> {
  // Records written before primaryDocId landed don't carry one.
  if (typeof rec.primaryDocId !== "string" || rec.primaryDocId.length === 0) {
    return null;
  }
  // Records written before the per-doc shape hold a single top-level
  // `{iv, wrappedDek}` — the Home DEK. Read it as the primary entry so
  // an anonymous session (whose local doc is unreachable without its
  // DEK) survives the upgrade; the next `save` rewrites the new shape.
  const legacy = rec as unknown as Partial<WrappedEntry>;
  const entries: Record<string, WrappedEntry> =
    rec.docKeys ??
    (legacy.iv && legacy.wrappedDek
      ? { [rec.primaryDocId]: { iv: legacy.iv, wrappedDek: legacy.wrappedDek } }
      : {});
  if (!entries[rec.primaryDocId]) return null;

  const docKeys = new Map<string, Dek>();
  for (const [docId, entry] of Object.entries(entries)) {
    docKeys.set(docId, await unwrapOne(rec.wrappingKey, entry, dekFromHex));
  }
  return {
    // Records written before anonymous mode landed have no flag;
    // they're authenticated by definition.
    anonymous: rec.anonymous ?? false,
    accountId: rec.accountId,
    primaryDocId: rec.primaryDocId,
    email: rec.email ?? null,
    deviceId: rec.deviceId ?? null,
    dek: docKeys.get(rec.primaryDocId)!,
    docKeys,
  };
}

async function wrapOne(wrappingKey: CryptoKey, dek: Dek): Promise<WrappedEntry> {
  const iv = crypto.getRandomValues(new Uint8Array(new ArrayBuffer(12)));
  const wrappedDek = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: "AES-GCM", iv },
      wrappingKey,
      fromHex(dek.toHex()),
    ),
  );
  return { iv, wrappedDek };
}

async function unwrapOne(
  wrappingKey: CryptoKey,
  entry: WrappedEntry,
  dekFromHex: DekFromHex,
): Promise<Dek> {
  const bytes = new Uint8Array(
    await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: copyToFreshBuffer(entry.iv) },
      wrappingKey,
      copyToFreshBuffer(entry.wrappedDek),
    ),
  );
  return dekFromHex(toHex(bytes));
}

// ---------- IndexedDB helpers ----------

function idbGet<T>(db: IDBDatabase, store: string, key: string): Promise<T | undefined> {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, "readonly");
    const req = tx.objectStore(store).get(key);
    req.onsuccess = () => resolve(req.result as T | undefined);
    req.onerror = () => reject(req.error);
  });
}

function idbPut(db: IDBDatabase, store: string, key: string, value: unknown): Promise<void> {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, "readwrite");
    tx.objectStore(store).put(value, key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}

/** Get, transform, put — in one readwrite transaction. `f` must be
 *  synchronous (an await would let the transaction auto-commit).
 *  Returning `undefined` leaves the store untouched. */
function idbUpdate<T>(
  db: IDBDatabase,
  store: string,
  key: string,
  f: (current: T | undefined) => T | undefined,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, "readwrite");
    const os = tx.objectStore(store);
    const req = os.get(key);
    req.onsuccess = () => {
      try {
        const next = f(req.result as T | undefined);
        if (next !== undefined) os.put(next, key);
      } catch (e) {
        reject(e);
        tx.abort();
      }
    };
    req.onerror = () => reject(req.error);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}

function idbDelete(db: IDBDatabase, store: string, key: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, "readwrite");
    tx.objectStore(store).delete(key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}

function toHex(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += b.toString(16).padStart(2, "0");
  return s;
}

function fromHex(hex: string): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(new ArrayBuffer(hex.length >>> 1));
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substr(i * 2, 2), 16);
  }
  return out;
}

/** Copy a Uint8Array into a fresh ArrayBuffer-backed Uint8Array. The
 *  WebCrypto `BufferSource` typedef rejects `Uint8Array<ArrayBufferLike>`
 *  (which IDB-roundtripped buffers carry), so we shake the SAB-aware
 *  marker off by copying. */
function copyToFreshBuffer(src: Uint8Array): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(new ArrayBuffer(src.byteLength));
  out.set(src);
  return out;
}
