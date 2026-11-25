// Attachments through the web store and the files host
// (`spec/attachments.md`): the doc half crosses the wasm boundary as a
// surgical `itemFilesChanged`, and the host half runs the client flows
// (upload first / reference second, open, purge, sweep, the copy before
// a cross-doc move) against a stand-in for the file routes.

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";
import { Dek, Doc, Replica, openFile, sealFile } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import { ApiError, type FilesList, type StoredFile } from "../src/api.ts";
import { fileCount, formatFileSize } from "../src/fileFormat.ts";
import {
  cleanFileName,
  createFilesHost,
  FileError,
  MAX_FILE_BYTES,
  type FileSource,
  type SweepStamps,
} from "../src/sync/files.ts";
import { createSyncedApp, type DocApp } from "../src/sync/store.ts";

const DOC_A = "00000000-0000-0000-0000-00000000000a";
const DOC_B = "00000000-0000-0000-0000-00000000000b";
const DAY = 24 * 60 * 60 * 1000;

function appOver(docId: string, doc: Doc = Doc.create()): DocApp {
  return createSyncedApp(
    new Replica(doc, docId, Dek.generate(), 0n, new MemReplicaStorage() as unknown as ReplicaStorage),
  );
}

const fid = (n: number): string => n.toString(16).padStart(32, "0");
const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);
const source = (name: string, text: string, type = "text/plain"): FileSource => {
  const body = bytes(text);
  return {
    name,
    type,
    size: body.length,
    arrayBuffer: async () => body.buffer.slice(0) as ArrayBuffer,
  };
};
const fail = (status: number): never => {
  throw new ApiError(status, "test", "req", `status ${status}`);
};

/** The file routes, in memory: one row map per doc. */
function fakeServer(opts: { owner?: boolean; now?: () => number } = {}) {
  const now = opts.now ?? Date.now;
  const rows = new Map<string, Map<string, StoredFile & { body: Uint8Array }>>();
  const doc = (docId: string) => {
    let m = rows.get(docId);
    if (!m) rows.set(docId, (m = new Map()));
    return m;
  };
  const calls: string[] = [];
  let down = false;
  let quota = Infinity;
  const guard = (call: string): void => {
    calls.push(call);
    if (down) throw new TypeError("network down");
  };
  return {
    rows: doc,
    calls,
    setDown: (b: boolean) => (down = b),
    setQuota: (n: number) => (quota = n),
    api: {
      async putFile(docId: string, fileId: string, body: Uint8Array) {
        guard(`put ${fileId}`);
        if (doc(docId).has(fileId)) fail(409);
        if (body.length > quota) fail(507);
        doc(docId).set(fileId, { id: fileId, bytes: body.length, created_at: now(), body });
      },
      async getFile(docId: string, fileId: string) {
        guard(`get ${fileId}`);
        const row = doc(docId).get(fileId) ?? fail(404);
        if (row.deleted_at != null) fail(410);
        return row.body;
      },
      async deleteFile(docId: string, fileId: string) {
        guard(`delete ${fileId}`);
        if (!doc(docId).delete(fileId)) fail(404);
      },
      async tombstoneFile(docId: string, fileId: string) {
        guard(`tombstone ${fileId}`);
        const row = doc(docId).get(fileId) ?? fail(404);
        row.deleted_at ??= now();
      },
      async restoreFile(docId: string, fileId: string) {
        guard(`restore ${fileId}`);
        const row = doc(docId).get(fileId) ?? fail(404);
        row.deleted_at = null;
      },
      async copyFile(docId: string, fileId: string, toDocId: string) {
        guard(`copy ${fileId}`);
        const row = doc(docId).get(fileId) ?? fail(404);
        if (row.deleted_at != null) fail(410);
        if (doc(toDocId).has(fileId)) fail(409);
        if (row.bytes > quota) fail(507);
        doc(toDocId).set(fileId, { ...row, created_at: now(), deleted_at: null });
      },
      async listFiles(docId: string): Promise<FilesList> {
        guard("list");
        const files = [...doc(docId).values()].map(({ body: _body, ...row }) => row);
        return {
          doc_bytes: 0,
          ...(opts.owner === false ? {} : { used_bytes: 0, limit_bytes: 1 }),
          files,
        };
      },
    },
  };
}

function memStamps(): SweepStamps & { map: Map<string, number> } {
  const map = new Map<string, number>();
  return { map, get: (id) => map.get(id) ?? null, set: (id, at) => void map.set(id, at) };
}

/** A host over `apps` and a fake server. Every doc counts as synced
 *  unless `unsynced` says otherwise: there is no server behind these
 *  replicas to acknowledge a push. */
function setup(apps: DocApp[], opts: { owner?: boolean; anonymous?: boolean } = {}) {
  let clock = 1_000_000_000_000;
  const unsynced = new Set<string>();
  for (const app of apps) app.hasUnsyncedOps = () => unsynced.has(app.docId);
  const server = fakeServer({ owner: opts.owner, now: () => clock });
  const stamps = memStamps();
  const host = createFilesHost({
    anonymous: opts.anonymous ?? false,
    appOf: (docId) => apps.find((a) => a.docId === docId),
    api: server.api,
    now: () => clock,
    sweepStamps: stamps,
  });
  return { host, server, stamps, unsynced, advance: (ms: number) => (clock += ms) };
}

const kindOf = async (p: Promise<unknown>): Promise<string> => {
  try {
    await p;
    return "resolved";
  } catch (e) {
    return e instanceof FileError ? e.kind : `threw ${String(e)}`;
  }
};

describe("attachments in the store", () => {
  test("attach, remove and undo", () => {
    createRoot((dispose) => {
      const app = appOver(DOC_A);
      const id = app.addItem("inbox", "trip");
      const names = () => (app.getItem(id)?.files ?? []).map((f) => f.name);
      expect(app.getItem(id)?.files).toBeUndefined();
      expect(fileCount(app.getItem(id)?.files)).toBeNull();

      const key = new Uint8Array(32).fill(7);
      app.attachFile(id, fid(1), " ticket.pdf ", "application/pdf", 1234, key);
      app.attachFile(id, fid(2), "map.png", "image/png", 99, key);
      expect(names().sort()).toEqual(["map.png", "ticket.pdf"]);
      expect(fileCount(app.getItem(id)?.files)).toBe(2);
      const ticket = app.getItem(id)!.files!.find((f) => f.id === fid(1))!;
      expect(ticket).toMatchObject({ mime: "application/pdf", size: 1234 });
      expect(app.fileKey(fid(1))).toEqual(key);

      // A removal hides the file and leaves its entry as the purge
      // mark, to be destroyed.
      app.removeFile(fid(2));
      expect(names()).toEqual(["ticket.pdf"]);
      expect(app.filesToPurge()).toEqual([fid(2)]);
      expect(app.filesToDestroy()).toEqual([fid(2)]);
      expect(app.fileKey(fid(2))).toBeUndefined();

      // It is permanent: undo reverses the edit before it (attaching
      // the map, whose entry stays marked or goes, never live again).
      app.undo();
      expect(names()).toEqual(["ticket.pdf"]);
      expect(app.fileKey(fid(2))).toBeUndefined();
      app.redo();
      expect(names()).toEqual(["ticket.pdf"]);

      // Forgetting a purged entry is not an undo step.
      expect(app.forgetFiles(app.filesToPurge())).toBeLessThanOrEqual(1);
      expect(app.storedFileIds()).toEqual([fid(1)]);
      dispose();
    });
  });

  test("files already in the doc arrive with the boot snapshot", () => {
    createRoot((dispose) => {
      const first = appOver(DOC_A);
      const id = first.addItem("inbox", "has a file");
      first.attachFile(id, fid(1), "notes.txt", "", 5, new Uint8Array(32));
      const snapshot = first.replica.save();
      const second = appOver(DOC_A, Doc.load(snapshot));
      expect(second.getItem(id)?.files?.map((f) => f.name)).toEqual(["notes.txt"]);
      dispose();
    });
  });
});

describe("files host", () => {
  test("attach uploads a sealed blob, then writes the entry; read opens it", async () => {
    const app = createRoot(() => appOver(DOC_A));
    const { host, server } = setup([app]);
    const id = app.addItem("inbox", "item");

    const fileId = await host.attach(app, id, source("hello.txt", "hello there"));
    const row = server.rows(DOC_A).get(fileId)!;
    expect(row.bytes).toBe("hello there".length + 40);
    expect(new TextDecoder().decode(row.body)).not.toContain("hello");
    expect(app.getItem(id)?.files).toMatchObject([
      { id: fileId, name: "hello.txt", mime: "text/plain", size: 11 },
    ]);
    expect(new TextDecoder().decode(await host.read(app, fileId))).toBe("hello there");
    // One undo step: the entry goes, the blob waits for the sweep.
    app.undo();
    expect(app.getItem(id)?.files).toBeUndefined();
    expect(await kindOf(host.read(app, fileId))).toBe("unavailable");
  });

  test("a failed upload leaves the doc untouched", async () => {
    const app = createRoot(() => appOver(DOC_A));
    const { host, server } = setup([app]);
    const id = app.addItem("inbox", "item");

    const big = { ...source("big.bin", ""), size: MAX_FILE_BYTES + 1 };
    expect(await kindOf(host.attach(app, id, big))).toBe("tooLarge");
    expect(server.calls).toEqual([]);

    server.setQuota(10);
    expect(await kindOf(host.attach(app, id, source("a.txt", "x")))).toBe("quota");
    server.setQuota(Infinity);
    server.setDown(true);
    expect(await kindOf(host.attach(app, id, source("a.txt", "x")))).toBe("offline");
    server.setDown(false);
    expect(await kindOf(host.attach(app, "no-such-item", source("a.txt", "x")))).toBe("failed");
    expect(app.getItem(id)?.files).toBeUndefined();
    expect(app.storedFileIds()).toEqual([]);

    const anon = setup([app], { anonymous: true });
    expect(await kindOf(anon.host.attach(app, id, source("a.txt", "x")))).toBe("signedOut");
    expect(anon.server.calls).toEqual([]);
  });

  test("a tombstoned blob the doc still names is restored on read", async () => {
    const app = createRoot(() => appOver(DOC_A));
    const { host, server } = setup([app]);
    const id = app.addItem("inbox", "item");
    const fileId = await host.attach(app, id, source("a.txt", "abc"));

    server.rows(DOC_A).get(fileId)!.deleted_at = 5;
    expect(new TextDecoder().decode(await host.read(app, fileId))).toBe("abc");
    expect(server.rows(DOC_A).get(fileId)!.deleted_at).toBeNull();

    // Purged for good, or a body that does not open: unavailable.
    const body = server.rows(DOC_A).get(fileId)!.body;
    server.rows(DOC_A).get(fileId)!.body = sealFile(new Uint8Array(32), bytes("other"));
    expect(await kindOf(host.read(app, fileId))).toBe("unavailable");
    server.rows(DOC_A).get(fileId)!.body = body;
    server.rows(DOC_A).delete(fileId);
    expect(await kindOf(host.read(app, fileId))).toBe("unavailable");
  });

  test("purge tombstones marked blobs and forgets their entries, once synced", async () => {
    const app = createRoot(() => appOver(DOC_A));
    const { host, server, stamps, unsynced } = setup([app]);
    stamps.set(DOC_A, 1_000_000_000_000); // swept today: purge only
    const keep = app.addItem("inbox", "keep");
    const bin = app.addItem("inbox", "bin me");
    const kept = await host.attach(app, keep, source("kept.txt", "k"));
    const removed = await host.attach(app, keep, source("removed.txt", "r"));
    const lost = await host.attach(app, bin, source("lost.txt", "l"));

    app.removeFile(removed);
    app.setBinned(bin, true);
    app.deleteBinned(bin);
    expect(app.filesToPurge().sort()).toEqual([removed, lost].sort());

    // The marks have not reached the server yet: nothing is deleted.
    unsynced.add(DOC_A);
    await host.settle(DOC_A);
    expect(server.calls.filter((c) => c.startsWith("delete"))).toEqual([]);

    // Both removals are deliberate: the server destroys the blobs.
    unsynced.clear();
    await host.settle(DOC_A);
    expect(server.rows(DOC_A).has(removed)).toBe(false);
    expect(server.rows(DOC_A).has(lost)).toBe(false);
    expect(server.rows(DOC_A).get(kept)!.deleted_at).toBeUndefined();
    expect(app.filesToPurge()).toEqual([]);
    expect(app.storedFileIds()).toEqual([kept]);

    // A blob already gone on the server counts as answered; a network
    // failure keeps the mark for the next pulse.
    const second = await host.attach(app, keep, source("second.txt", "s"));
    app.removeFile(second);
    server.setDown(true);
    await host.settle(DOC_A);
    expect(app.filesToPurge()).toEqual([second]);
    server.setDown(false);
    server.rows(DOC_A).delete(second);
    await host.settle(DOC_A);
    expect(app.filesToPurge()).toEqual([]);
  });

  test("a pulse purges at once when the mark is synced, and waits otherwise", async () => {
    const app = createRoot(() => appOver(DOC_A));
    const { host, server, stamps, unsynced } = setup([app]);
    stamps.set(DOC_A, 1_000_000_000_000); // swept today: purge only
    const item = app.addItem("inbox", "item");
    const first = await host.attach(app, item, source("first.txt", "1"));
    const second = await host.attach(app, item, source("second.txt", "2"));
    const deletes = () => server.calls.filter((c) => c.startsWith("delete")).length;
    const turn = () => new Promise((r) => setTimeout(r, 0));

    // The mark is still local: the pulse only arms the settle timer.
    app.removeFile(first);
    unsynced.add(DOC_A);
    host.pulse(DOC_A);
    await turn();
    expect(deletes()).toBe(0);

    // The ack's pulse: sent without waiting out the delay.
    unsynced.clear();
    host.pulse(DOC_A);
    await turn();
    expect(server.rows(DOC_A).has(first)).toBe(false);
    expect(app.filesToPurge()).toEqual([]);

    // After a failed request the next pulse goes back to the delay.
    app.removeFile(second);
    server.setDown(true);
    host.pulse(DOC_A);
    await turn();
    server.setDown(false);
    const before = deletes();
    host.pulse(DOC_A);
    await turn();
    expect(deletes()).toBe(before);
    expect(app.filesToPurge()).toEqual([second]);
    await host.settle(DOC_A);
    expect(app.filesToPurge()).toEqual([]);
    host.dispose();
  });

  test("the owner's sweep tombstones day-old blobs no entry names, once a day", async () => {
    const app = createRoot(() => appOver(DOC_A));
    const { host, server, stamps, advance } = setup([app]);
    const id = app.addItem("inbox", "item");
    const named = await host.attach(app, id, source("named.txt", "n"));
    const undone = await host.attach(app, id, source("undone.txt", "u"));
    app.undo(); // the attach: an orphan blob
    const marked = await host.attach(app, id, source("marked.txt", "m"));
    app.removeFile(marked);
    const deleted = (fileId: string) => server.rows(DOC_A).get(fileId)!.deleted_at != null;

    // Too young: an upload may still be on its way to its commit.
    stamps.set(DOC_A, 0);
    await host.settle(DOC_A);
    expect(deleted(undone)).toBe(false);
    expect(stamps.get(DOC_A)).toBe(1_000_000_000_000);

    // Within the day: not swept again, whatever its age.
    advance(DAY - 1);
    await host.settle(DOC_A);
    expect(server.calls.filter((c) => c === "list").length).toBe(1);

    advance(2);
    await host.settle(DOC_A);
    expect(deleted(undone)).toBe(true);
    expect(deleted(named)).toBe(false);
    // The marked one went by the purge, in the first settle: a
    // deliberate removal, so the server destroyed it.
    expect(server.rows(DOC_A).has(marked)).toBe(false);
    expect(app.storedFileIds()).toEqual([named]);
  });

  test("a member who is not the owner never sweeps", async () => {
    const app = createRoot(() => appOver(DOC_A));
    const { host, server, advance } = setup([app], { owner: false });
    const id = app.addItem("inbox", "item");
    const orphan = await host.attach(app, id, source("orphan.txt", "o"));
    app.undo();
    advance(2 * DAY);
    await host.settle(DOC_A);
    expect(server.calls).toContain("list");
    expect(server.rows(DOC_A).get(orphan)!.deleted_at).toBeUndefined();
    // Asked once for the day.
    await host.settle(DOC_A);
    expect(server.calls.filter((c) => c === "list").length).toBe(1);
  });

  test("copy for a cross-doc move, and the move back", async () => {
    const [a, b] = createRoot(() => [appOver(DOC_A), appOver(DOC_B)]);
    const { host, server } = setup([a!, b!]);
    const id = a!.addItem("inbox", "moving");
    const fileId = await host.attach(a!, id, source("doc.txt", "contents"));

    await host.copyForMove([{ docId: DOC_A, fileId }], DOC_B);
    b!.adoptItems(a!.exportItemsJson([id]), "inbox", 0);
    a!.releaseItems([id]);
    expect(b!.getItem(id)?.files?.map((f) => f.name)).toEqual(["doc.txt"]);
    expect(new TextDecoder().decode(await host.read(b!, fileId))).toBe("contents");
    // The source's entry is marked, and its blob goes by the purge.
    expect(a!.filesToPurge()).toEqual([fileId]);
    await host.settle(DOC_A);
    expect(server.rows(DOC_A).get(fileId)!.deleted_at).not.toBeNull();

    // Back again: the id is still on the source's server rows.
    await host.copyForMove([{ docId: DOC_B, fileId }], DOC_A);
    expect(server.rows(DOC_A).get(fileId)!.deleted_at).toBeNull();
    a!.adoptItems(b!.exportItemsJson([id]), "inbox", 0);
    b!.releaseItems([id]);
    expect(new TextDecoder().decode(await host.read(a!, fileId))).toBe("contents");

    // A copy that cannot happen stops the move.
    server.setQuota(1);
    const other = fid(9);
    server.rows(DOC_A).set(other, { id: other, bytes: 50, created_at: 0, body: bytes("x") });
    expect(await kindOf(host.copyForMove([{ docId: DOC_A, fileId: other }], DOC_B))).toBe("quota");
    server.setDown(true);
    expect(await kindOf(host.copyForMove([{ docId: DOC_A, fileId: other }], DOC_B))).toBe(
      "offline",
    );
  });
});

describe("file helpers", () => {
  test("sealed bodies open only with their key", () => {
    const key = new Uint8Array(32).fill(1);
    const blob = sealFile(key, bytes("secret"));
    expect(blob.length).toBe(6 + 40);
    expect(new TextDecoder().decode(openFile(key, blob))).toBe("secret");
    expect(() => openFile(new Uint8Array(32).fill(2), blob)).toThrow();
  });

  test("names are cleaned to what the core accepts", () => {
    expect(cleanFileName("  report.pdf \n")).toBe("report.pdf");
    expect(cleanFileName("a\u0000b\u0007.txt")).toBe("ab.txt");
    expect(cleanFileName("\t ")).toBe("file");
    const long = cleanFileName("é".repeat(300) + ".jpeg");
    expect(new TextEncoder().encode(long).length).toBeLessThanOrEqual(255);
    expect(long.endsWith(".jpeg")).toBe(true);
  });

  test("sizes", () => {
    expect(formatFileSize(0, "en-US")).toBe("0 B");
    expect(formatFileSize(1023, "en-US")).toBe("1,023 B");
    expect(formatFileSize(2048, "en-US")).toBe("2 KB");
    expect(formatFileSize(5 * 1024 * 1024 + 300_000, "en-US")).toBe("5.3 MB");
    expect(formatFileSize(1024 * 1024 * 1024, "en-US")).toBe("1 GB");
  });
});
