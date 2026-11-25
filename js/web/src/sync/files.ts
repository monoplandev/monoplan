// The host half of attachments (`spec/attachments.md` "Client flows"):
// everything that moves bytes or talks to the file routes. The doc half
// (entries, keys, purge marks) is the core's, reached through each
// `DocApp`; this module sits between it and the server:
//
//   - attach: seal under a fresh key, `PUT`, then write the entry
//     (upload first, reference second),
//   - read: `GET`, open with the entry's key; a tombstoned blob the doc
//     still names is restored and fetched again,
//   - copy for a cross-doc move: the server duplicates each blob into
//     the target doc before the entries move,
//   - purge: remove the blobs of the entries marked for purge (destroyed
//     at once for a deliberate removal, tombstoned for the rest), then
//     delete those entries,
//   - sweep (the doc owner's clients, once a day per doc): mark entries
//     whose item is gone, tombstone blobs no entry names.
//
// Purge and sweep are housekeeping. The runtime pulses a doc after each
// round-trip with the server and when the socket comes up. A pulse that
// finds entries marked for purge and nothing unsent settles at once, so
// a removal reaches the server as soon as its mark has; any other pulse
// settles a few seconds after the last one. Nothing acts while the
// doc's own changes are still local, so a removal is never acted on
// before the mark that explains it is on the server.
//
// Anonymous sessions have no server, so nothing here runs for them and
// attaching is refused.

import { openFile, sealFile } from "@monoplan/core/wasm";
import { ApiError, api as httpApi } from "../api.ts";
import type { DocApp } from "./store.ts";

/** Largest file that can be attached: the server's cap on the
 *  plaintext (`spec/attachments.md` §Routes). */
export const MAX_FILE_BYTES = 25 * 1024 * 1024;

const DAY_MS = 24 * 60 * 60 * 1000;
/** How long after the last pulse a doc's housekeeping runs, when there
 *  is no synced purge mark to act on at once. */
const SETTLE_MS = 3000;
const FILE_NAME_MAX_BYTES = 255;

/** Why a file operation did not happen, in terms the UI can word:
 *  `tooLarge` (over the cap), `quota` (the workspace owner's storage is
 *  full), `offline` (the request never reached the server),
 *  `signedOut` (an anonymous session), `unavailable` (the server no
 *  longer has the bytes, or they do not open), `failed` (anything
 *  else). */
export type FileErrorKind =
  | "tooLarge"
  | "quota"
  | "offline"
  | "signedOut"
  | "unavailable"
  | "failed";

export class FileError extends Error {
  constructor(
    public readonly kind: FileErrorKind,
    cause?: unknown,
  ) {
    super(`file: ${kind}`, { cause });
    this.name = "FileError";
  }
}

/** What `attach` needs of a browser `File`. */
export interface FileSource {
  name: string;
  type: string;
  size: number;
  arrayBuffer(): Promise<ArrayBuffer>;
}

/** When each doc was last swept, so a doc is swept at most once a day
 *  on this device. */
export interface SweepStamps {
  get(docId: string): number | null;
  set(docId: string, at: number): void;
}

type FilesApi = Pick<
  typeof httpApi,
  | "putFile"
  | "getFile"
  | "deleteFile"
  | "tombstoneFile"
  | "restoreFile"
  | "copyFile"
  | "listFiles"
>;

export interface FilesHost {
  /** False on an anonymous session: no attach, no housekeeping. */
  readonly enabled: boolean;
  /** Upload `file` and attach it to `itemId` of `app`. Resolves with
   *  the new file id once the entry is in the doc; rejects with a
   *  `FileError` and leaves the doc untouched otherwise. */
  attach(app: DocApp, itemId: string, file: FileSource): Promise<string>;
  /** The plaintext bytes of a live attachment of `app`. Rejects with a
   *  `FileError`. Nothing is kept: every call fetches. */
  read(app: DocApp, fileId: string): Promise<Uint8Array>;
  /** Before a cross-doc move: have the server copy each blob into
   *  `targetDocId` under its own id. Rejects with a `FileError` on the
   *  first copy that fails, and the caller then does not move anything. */
  copyForMove(
    files: readonly { docId: string; fileId: string }[],
    targetDocId: string,
  ): Promise<void>;
  /** Something happened on `docId` (a server round-trip, the socket
   *  coming up): run its housekeeping once things go quiet. */
  pulse(docId: string): void;
  /** Run a doc's purge and sweep now. Resolves when both are done;
   *  never rejects. */
  settle(docId: string): Promise<void>;
  /** The doc closed: drop its pending housekeeping. */
  forget(docId: string): void;
  dispose(): void;
}

const SWEEP_KEY = "monoplan.files.sweep.";

const localSweepStamps: SweepStamps = {
  get(docId) {
    try {
      const raw = localStorage.getItem(SWEEP_KEY + docId);
      const at = raw === null ? NaN : Number(raw);
      return Number.isFinite(at) ? at : null;
    } catch {
      return null;
    }
  },
  set(docId, at) {
    try {
      localStorage.setItem(SWEEP_KEY + docId, String(at));
    } catch {
      // Private mode: the doc is swept again next session, which is
      // harmless.
    }
  },
};

const statusOf = (e: unknown): number | null => (e instanceof ApiError ? e.status : null);

function classify(e: unknown): FileError {
  if (e instanceof FileError) return e;
  if (!(e instanceof ApiError)) return new FileError("offline", e);
  switch (e.status) {
    case 413:
      return new FileError("tooLarge", e);
    case 507:
      return new FileError("quota", e);
    case 404:
    case 410:
      return new FileError("unavailable", e);
    default:
      return new FileError("failed", e);
  }
}

function randomHex(bytes: number): string {
  const buf = crypto.getRandomValues(new Uint8Array(bytes));
  let out = "";
  for (const b of buf) out += b.toString(16).padStart(2, "0");
  return out;
}

const utf8Length = (s: string): number => new TextEncoder().encode(s).length;

/** A name the core accepts for what the browser reported: control
 *  characters out, trimmed, and cut to 255 bytes with the extension
 *  kept. Empty comes back as `fallback`. */
export function cleanFileName(raw: string, fallback = "file"): string {
  const name = raw.replace(/[\u0000-\u001f\u007f-\u009f]/g, "").trim();
  if (!name) return fallback;
  if (utf8Length(name) <= FILE_NAME_MAX_BYTES) return name;
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 && name.length - dot <= 16 ? name.slice(dot) : "";
  let stem = [...name.slice(0, name.length - ext.length)];
  while (stem.length > 1 && utf8Length(stem.join("").trimEnd() + ext) > FILE_NAME_MAX_BYTES) {
    stem = stem.slice(0, -1);
  }
  return stem.join("").trimEnd() + ext;
}

export function createFilesHost(opts: {
  anonymous: boolean;
  /** The open doc with this id, if it is still open. */
  appOf: (docId: string) => DocApp | undefined;
  api?: FilesApi;
  now?: () => number;
  sweepStamps?: SweepStamps;
}): FilesHost {
  const api = opts.api ?? httpApi;
  const now = opts.now ?? Date.now;
  const stamps = opts.sweepStamps ?? localSweepStamps;
  const enabled = !opts.anonymous;
  const isOpen = (app: DocApp): boolean => opts.appOf(app.docId) === app;

  const attach = async (app: DocApp, itemId: string, file: FileSource): Promise<string> => {
    if (!enabled) throw new FileError("signedOut");
    if (file.size > MAX_FILE_BYTES) throw new FileError("tooLarge");
    if (!app.getItem(itemId)) throw new FileError("failed");
    const name = cleanFileName(file.name);
    const mime = utf8Length(file.type) <= 255 ? file.type : "";
    let plain: Uint8Array;
    try {
      plain = new Uint8Array(await file.arrayBuffer());
    } catch (e) {
      throw new FileError("failed", e);
    }
    // The size the browser reported can be stale (the file changed on
    // disk); the bytes read are what counts.
    if (plain.length > MAX_FILE_BYTES) throw new FileError("tooLarge");
    const key = crypto.getRandomValues(new Uint8Array(32));
    const fileId = randomHex(16);
    const sealed = sealFile(key, plain);
    try {
      await api.putFile(app.docId, fileId, sealed);
    } catch (e) {
      // 409: the id is random and ours, so the blob is already there.
      if (statusOf(e) !== 409) throw classify(e);
    }
    // Reference second. If the item or the doc went away during the
    // upload the blob stays unnamed and the owner's sweep collects it.
    if (!isOpen(app)) throw new FileError("failed");
    try {
      app.attachFile(itemId, fileId, name, mime, plain.length, key);
    } catch (e) {
      throw new FileError("failed", e);
    }
    return fileId;
  };

  const read = async (app: DocApp, fileId: string): Promise<Uint8Array> => {
    if (!enabled) throw new FileError("signedOut");
    const key = app.fileKey(fileId);
    if (!key) throw new FileError("unavailable");
    let sealed: Uint8Array;
    try {
      sealed = await api.getFile(app.docId, fileId);
    } catch (e) {
      if (statusOf(e) !== 410) throw classify(e);
      // The doc still names a blob the server has tombstoned (an undone
      // removal, a stale device's delete): the reference wins inside
      // the grace period.
      try {
        await api.restoreFile(app.docId, fileId);
        sealed = await api.getFile(app.docId, fileId);
      } catch (e2) {
        throw statusOf(e2) === null ? classify(e2) : new FileError("unavailable", e2);
      }
    }
    try {
      return openFile(key, sealed);
    } catch (e) {
      throw new FileError("unavailable", e);
    }
  };

  const copyForMove = async (
    files: readonly { docId: string; fileId: string }[],
    targetDocId: string,
  ): Promise<void> => {
    if (files.length === 0) return;
    if (!enabled) throw new FileError("signedOut");
    for (const { docId, fileId } of files) {
      if (docId === targetDocId) continue;
      try {
        try {
          await api.copyFile(docId, fileId, targetDocId);
        } catch (e) {
          if (statusOf(e) !== 410) throw e;
          // Tombstoned at the source though the doc names it: restore,
          // then copy.
          await api.restoreFile(docId, fileId);
          await api.copyFile(docId, fileId, targetDocId);
        }
      } catch (e) {
        const status = statusOf(e);
        if (status === 409) {
          // The target already has the id: the item is moving back to a
          // doc it left. Its blob there may be tombstoned by now.
          try {
            await api.restoreFile(targetDocId, fileId);
          } catch (e2) {
            throw classify(e2);
          }
        } else if (status === 404) {
          // The source has no such blob: the file was already
          // unavailable, and stays so after the move.
        } else {
          throw classify(e);
        }
      }
    }
  };

  // Remove the blob of every entry marked for purge, then delete the
  // entries the server answered for. A deliberate removal (the file's
  // own delete, a hard-deleted item) has the server destroy the bytes
  // now; a released or ownerless entry is only tombstoned. Stops at the first request that
  // fails for another reason; the next pulse tries again. False when
  // it stopped that way.
  const purge = async (app: DocApp): Promise<boolean> => {
    const marked = app.filesToPurge();
    if (marked.length === 0) return true;
    const destroy = new Set(app.filesToDestroy());
    const gone: string[] = [];
    let complete = true;
    for (const fileId of marked) {
      try {
        if (destroy.has(fileId)) await api.deleteFile(app.docId, fileId);
        else await api.tombstoneFile(app.docId, fileId);
        gone.push(fileId);
      } catch (e) {
        const status = statusOf(e);
        if (status !== 404 && status !== 410) {
          if (status !== null) console.warn("files: purge stopped:", e);
          complete = false;
          break;
        }
        gone.push(fileId);
      }
    }
    if (gone.length > 0 && isOpen(app)) app.forgetFiles(gone);
    return complete;
  };

  // Orphans (`spec/attachments.md` "Garbage"). Only the doc owner's
  // clients sweep, which the list answer tells: the pooled quota
  // figures come to the owner alone.
  const sweep = async (app: DocApp): Promise<void> => {
    const last = stamps.get(app.docId);
    const at = now();
    if (last !== null && at - last < DAY_MS && at >= last) return;
    let list;
    try {
      list = await api.listFiles(app.docId);
    } catch (e) {
      if (statusOf(e) !== null) console.warn("files: sweep skipped:", e);
      return;
    }
    if (!isOpen(app)) return;
    const owner = list.used_bytes != null;
    // The doc may have moved on while the list was in flight: an edit
    // of this device's own that the server has not seen postpones the
    // sweep to the next pulse.
    if (owner && app.hasUnsyncedOps()) return;
    stamps.set(app.docId, at);
    if (!owner) return;
    const cutoff = at - DAY_MS;
    app.markOwnerlessFiles(cutoff);
    const named = new Set(app.storedFileIds());
    for (const row of list.files) {
      if (row.deleted_at != null || row.created_at >= cutoff || named.has(row.id)) continue;
      try {
        await api.tombstoneFile(app.docId, row.id);
      } catch (e) {
        if (statusOf(e) === null) return;
      }
    }
  };

  const running = new Set<string>();
  const again = new Set<string>();
  // Docs whose last purge stopped on a failed request: their pulses
  // wait out the settle delay again, so a server that keeps failing is
  // not asked once per frame.
  const stalled = new Set<string>();
  const timers = new Map<string, ReturnType<typeof setTimeout>>();

  const settle = async (docId: string): Promise<void> => {
    if (!enabled) return;
    if (running.has(docId)) {
      again.add(docId);
      return;
    }
    const app = opts.appOf(docId);
    // Unsent changes: the push's ack is a server frame, which pulses
    // this doc again.
    if (!app || app.hasUnsyncedOps()) return;
    running.add(docId);
    try {
      if (await purge(app)) stalled.delete(docId);
      else stalled.add(docId);
      if (isOpen(app)) await sweep(app);
    } catch (e) {
      console.warn("files: housekeeping failed:", e);
    } finally {
      running.delete(docId);
      if (again.delete(docId)) pulse(docId);
    }
  };

  const pulse = (docId: string): void => {
    if (!enabled) return;
    clearTimeout(timers.get(docId));
    timers.delete(docId);
    // A purge mark the server already has: act on it now.
    const app = opts.appOf(docId);
    if (
      app &&
      !stalled.has(docId) &&
      !app.hasUnsyncedOps() &&
      app.filesToPurge().length > 0
    ) {
      void settle(docId);
      return;
    }
    timers.set(
      docId,
      setTimeout(() => {
        timers.delete(docId);
        void settle(docId);
      }, SETTLE_MS),
    );
  };

  const forget = (docId: string): void => {
    clearTimeout(timers.get(docId));
    timers.delete(docId);
    again.delete(docId);
    stalled.delete(docId);
  };

  return {
    enabled,
    attach,
    read,
    copyForMove,
    pulse,
    settle,
    forget,
    dispose() {
      for (const t of timers.values()) clearTimeout(t);
      timers.clear();
      again.clear();
      stalled.clear();
    },
  };
}
