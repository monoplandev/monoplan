// End-to-end: real `monoplan-server` process, two wasm SyncEngines,
// real msgpack over a real WebSocket. Exercises the server-orchestrated
// snapshot path: device A crosses the (test-tuned) snapshot threshold,
// server requests a snapshot, A uploads it, device B then bootstraps
// from that snapshot blob rather than replaying every op.
//
// Why this test runs a real server: the Rust integration test in
// `server/tests/sync.rs` already covers the snapshot state machine
// from inside the server crate. This test is here to prove the *wasm
// surface* + real WS transport behave the same — catching e.g. a
// regression in `popOutbox()` ordering or `handleServerBytes()` that
// the pure-Rust test can't see.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { Database } from "bun:sqlite";
import { decode, encode } from "@msgpack/msgpack";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
  Dek,
  deriveLogin,
  deriveRecovery,
  Doc,
  generateRecoveryCode,
  helloFrame,
  parseHelloResponse,
  serverFrameDocId,
  SyncEngine,
  wrapDek,
} from "../wasm/monoplan_core_web.js";
import type { EngineStorage } from "../wasm/monoplan_core_web.js";
import { SyncBridge } from "../src/sync-bridge.ts";
import { MemEngineStorage } from "./mem-engine-storage.ts";

const REPO_ROOT = resolve(import.meta.dir, "../../..");
const SERVER_BIN = join(REPO_ROOT, "target/debug/monoplan-server");

const LIST_MAIN = "inbox";
const SNAPSHOT_THRESHOLD = 5;
// Weak Argon2 keeps signup ~ms; matches `server/tests/sync.rs::weak_params`.
const KDF = { m_kib: 8, t: 1, p: 1 } as const;

interface ServerHandle {
  baseUrl: string;
  wsUrl: string;
  dbPath: string;
  proc: Subprocess;
  workDir: string;
}

async function ensureBuilt(): Promise<void> {
  const proc = spawn(["cargo", "build", "-p", "monoplan-server"], {
    cwd: REPO_ROOT,
    stdout: "inherit",
    stderr: "inherit",
  });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`cargo build monoplan-server failed (exit ${code})`);
}

async function startServer(): Promise<ServerHandle> {
  const workDir = mkdtempSync(join(tmpdir(), "monoplan-e2e-"));
  const dbPath = join(workDir, "monoplan.sqlite");
  const proc = spawn([SERVER_BIN, "--bind", "127.0.0.1:0", "--db", dbPath], {
    cwd: workDir,
    env: {
      ...process.env,
      MONOPLAN_SNAPSHOT_THRESHOLD_BLOBS: String(SNAPSHOT_THRESHOLD),
      MONOPLAN_SECURE_COOKIES: "false",
      MONOPLAN_LOG_LEVEL: "info",
      // tracing-subscriber emits ANSI even on piped output; off so
      // `readUntilListening`'s regex sees clean bytes.
      NO_COLOR: "1",
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  const addr = await readUntilListening(proc);
  return {
    baseUrl: `http://${addr}`,
    wsUrl: `ws://${addr}`,
    dbPath,
    proc,
    workDir,
  };
}

// `monoplan-server` logs `monoplan-server listening addr=…` via tracing;
// we tee both streams to our own stderr so the test output stays
// useful when something blows up mid-handshake. We keep both
// consumers running for the life of the process so the server's
// stdio doesn't back-pressure — but the first match resolves.
async function readUntilListening(proc: Subprocess): Promise<string> {
  const re = /addr=(\d+\.\d+\.\d+\.\d+:\d+)/;
  const stripAnsi = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "");
  const decoder = new TextDecoder();
  let buf = "";
  let resolved = false;

  return new Promise((resolveOk, rejectOk) => {
    const consume = async (
      stream: ReadableStream<Uint8Array> | null,
    ): Promise<void> => {
      if (!stream) return;
      try {
        // Bun's ReadableStream is async-iterable; DOM types don't say so.
        for await (const chunk of stream as unknown as AsyncIterable<Uint8Array>) {
          const text = decoder.decode(chunk);
          process.stderr.write(text);
          if (resolved) continue;
          buf += stripAnsi(text);
          const m = buf.match(re);
          if (m) {
            resolved = true;
            resolveOk(m[1]);
          }
        }
      } catch {
        // Stream ended; the Promise.all .then() below handles the
        // "exited without listening" case.
      }
    };
    Promise.all([
      consume(proc.stderr as unknown as ReadableStream<Uint8Array> | null),
      consume(proc.stdout as unknown as ReadableStream<Uint8Array> | null),
    ]).then(() => {
      if (!resolved) {
        rejectOk(new Error("server exited before logging listen addr"));
      }
    });
  });
}

async function stopServer(s: ServerHandle): Promise<void> {
  s.proc.kill();
  await s.proc.exited;
  rmSync(s.workDir, { recursive: true, force: true });
}

interface Account {
  accountId: string;
  deviceToken: string;
  primaryDocId: string;
}

/** Signup enrols a recovery code unconditionally; this suite never
 *  redeems it, so only the material is built. */
function enrolRecovery(dek: Dek) {
  const code = generateRecoveryCode();
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const d = deriveRecovery(code, salt, KDF.m_kib, KDF.t, KDF.p);
  const w = wrapDek(d.recoveryKek, dek);
  return {
    recovery_salt: salt,
    recovery_auth_secret: d.recoveryAuthSecret,
    recovery_wrapped_dek: w.ciphertext,
    recovery_wrapped_dek_nonce: w.nonce,
  };
}

async function signup(s: ServerHandle, dekForWrap: Dek): Promise<Account> {
  const masterSalt = crypto.getRandomValues(new Uint8Array(16));
  const derived = deriveLogin(
    "test-password",
    masterSalt,
    KDF.m_kib,
    KDF.t,
    KDF.p,
  );
  const wrapped = wrapDek(derived.kek, dekForWrap);
  const email = `e2e-${crypto.randomUUID()}@example.com`;
  const body = encode({
    email,
    master_salt: masterSalt,
    kdf_params: KDF,
    auth_secret: derived.authSecret,
    wrapped_dek: wrapped.ciphertext,
    wrapped_dek_nonce: wrapped.nonce,
    recovery: enrolRecovery(dekForWrap),
    device_name: "device-a",
    device_kind: "web",
  });
  const resp = await fetch(`${s.baseUrl}/api/account/signup`, {
    method: "POST",
    headers: { "Content-Type": "application/msgpack" },
    body,
  });
  if (!resp.ok) {
    throw new Error(`signup failed: ${resp.status} ${await resp.text()}`);
  }
  const decoded = decode(new Uint8Array(await resp.arrayBuffer())) as {
    account_id: string;
    device_token: string;
    device_id: string;
    primary_doc_id: string;
  };
  return {
    accountId: decoded.account_id,
    deviceToken: decoded.device_token,
    primaryDocId: decoded.primary_doc_id,
  };
}

async function registerDevice(
  s: ServerHandle,
  primaryToken: string,
  name: string,
): Promise<string> {
  const body = encode({ name, kind: "web" });
  const resp = await fetch(`${s.baseUrl}/api/devices`, {
    method: "POST",
    headers: {
      "Content-Type": "application/msgpack",
      Authorization: `Bearer ${primaryToken}`,
    },
    body,
  });
  if (!resp.ok) {
    throw new Error(`register device failed: ${resp.status} ${await resp.text()}`);
  }
  const decoded = decode(new Uint8Array(await resp.arrayBuffer())) as {
    device_token: string;
  };
  return decoded.device_token;
}

/** `POST /api/docs`: a second doc the account owns. The wrap (nominally
 *  the doc's DEK under the Home DEK) is opaque to the server, so random
 *  bytes do for a test that keeps the DEK in memory. */
async function createDoc(s: ServerHandle, token: string): Promise<string> {
  const body = encode({
    wrapped_dek: crypto.getRandomValues(new Uint8Array(48)),
    wrapped_dek_nonce: crypto.getRandomValues(new Uint8Array(24)),
  });
  const resp = await fetch(`${s.baseUrl}/api/docs`, {
    method: "POST",
    headers: {
      "Content-Type": "application/msgpack",
      Authorization: `Bearer ${token}`,
    },
    body,
  });
  if (!resp.ok) {
    throw new Error(`create doc failed: ${resp.status} ${await resp.text()}`);
  }
  const decoded = decode(new Uint8Array(await resp.arrayBuffer())) as {
    doc_id: string;
  };
  return decoded.doc_id;
}

// Construct a bridge over `engines` with a bearer-header socket
// factory and resolve once it's seen "online" at least once. Tests
// then drive mutations and let the bridge's own onmessage pump push
// outbox bytes; explicit `pumpOutbox()` calls cover the case where
// local mutations happen between server frames. Any bridge-level
// error (bad handshake, unroutable frame) fails the test loudly.
function attachEngines(
  s: ServerHandle,
  token: string,
  engines: SyncEngine[],
): Promise<SyncBridge> {
  return new Promise((resolveOk, rejectOk) => {
    let resolved = false;
    const bridge = new SyncBridge({
      engines,
      codec: { helloFrame, parseHelloResponse, serverFrameDocId },
      clientName: "monoplan-e2e",
      clientVersion: "0.0.0",
      // Bun extends `new WebSocket` with an options object that
      // supports `headers`; the stdlib type sig only knows about
      // subprotocols.
      socketFactory: () =>
        new WebSocket(`${s.wsUrl}/api/sync`, {
          headers: { Authorization: `Bearer ${token}` },
        } as unknown as string[]),
      onChange: (kind) => {
        if (kind === "online" && !resolved) {
          resolved = true;
          resolveOk(bridge);
        }
      },
      onError: (message) => rejectOk(new Error(`bridge error: ${message}`)),
      onEngineEvent: (ev, docId) => {
        if (ev.kind === "error") {
          rejectOk(new Error(`engine error on doc ${docId}: ${ev.message}`));
        }
      },
      reconnectBackoff: { baseMs: 50, maxMs: 50, jitter: 0 },
    });
    bridge.start();
  });
}

function itemTexts(engine: SyncEngine): string[] {
  const items = JSON.parse(engine.itemsInListJson(LIST_MAIN, false)) as Array<{
    text: string;
  }>;
  return items.map((i) => i.text).sort();
}

async function waitFor(
  predicate: () => boolean | Promise<boolean>,
  label: string,
  timeoutMs = 5000,
): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await predicate()) return;
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error(`waitFor(${label}) timed out after ${timeoutMs}ms`);
}

function snapshotCount(dbPath: string): number {
  const db = new Database(dbPath, { readonly: true });
  try {
    const row = db.query("SELECT COUNT(*) as n FROM snapshots").get() as {
      n: number;
    };
    return row.n;
  } finally {
    db.close();
  }
}

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

describe("e2e snapshot + second-device bootstrap", () => {
  let server: ServerHandle;

  beforeAll(async () => {
    await ensureBuilt();
    server = await startServer();
  }, 180_000);

  afterAll(async () => {
    if (server) await stopServer(server);
  });

  test("device B converges with device A after snapshot path", async () => {
    const seedDek = Dek.generate();
    const account = await signup(server, seedDek.clone());

    const engineA = new SyncEngine(
      Doc.create(),
      account.primaryDocId,
      seedDek.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const bridgeA = await attachEngines(server, account.deviceToken, [engineA]);

    // Cross the threshold (5) by a margin so we get a SnapshotRequest
    // without ambiguity about off-by-one in the eligibility check.
    // Push one item at a time and wait for the server ack between
    // each — the engine bundles co-pending mutations into a single op
    // blob (= one server-assigned seq), so a tight batch wouldn't
    // accumulate enough seqs to cross the threshold.
    const ITEMS = SNAPSHOT_THRESHOLD + 3;
    for (let i = 0; i < ITEMS; i++) {
      engineA.addItem(LIST_MAIN, `item ${i}`);
      engineA.captureLocalOps();
      engineA.flush();
      bridgeA.pumpOutbox();
      await waitFor(
        () => engineA.lastContiguousSeq() >= BigInt(i + 1),
        `op ${i + 1} acked by server`,
      );
    }

    // The snapshot row landing in sqlite is the cleanest "the whole
    // server-orchestrated path completed" signal — server received
    // PushSnapshot from the engine and committed it.
    await waitFor(() => snapshotCount(server.dbPath) > 0, "snapshot row", 10_000);

    // Device B comes online fresh. Its `last_acked_seq=0` is below
    // the server's snapshot floor, so the server replies with
    // `SnapshotRequired` rather than streaming every op. The engine
    // transparently fetches the snapshot, loads it, and resumes.
    const tokenB = await registerDevice(server, account.deviceToken, "device-b");
    const engineB = new SyncEngine(
      Doc.empty(),
      account.primaryDocId,
      seedDek.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const bridgeB = await attachEngines(server, tokenB, [engineB]);

    await waitFor(
      () => hex(engineA.fingerprint()) === hex(engineB.fingerprint()),
      "fingerprint convergence",
      10_000,
    );

    expect(hex(engineB.fingerprint())).toBe(hex(engineA.fingerprint()));
    expect(itemTexts(engineB)).toEqual(
      Array.from({ length: ITEMS }, (_, i) => `item ${i}`).sort(),
    );

    bridgeA.stop();
    bridgeB.stop();
  }, 30_000);

  // Two private docs on one account, one socket per device. Proves the
  // whole multiplexed path end to end: one `Hello` per socket, an
  // `OpenDoc` per engine, inbound frames routed by `doc_id`, per-doc
  // seqs and acks, and no bleed between docs — on the real server, the
  // real wasm surface, and the real transport.
  test("two private docs sync over one connection per device without bleed", async () => {
    const dekHome = Dek.generate();
    const dekSide = Dek.generate();
    const account = await signup(server, dekHome.clone());
    const sideDocId = await createDoc(server, account.deviceToken);
    expect(sideDocId).not.toBe(account.primaryDocId);

    const homeA = new SyncEngine(
      Doc.create(),
      account.primaryDocId,
      dekHome.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const sideA = new SyncEngine(
      Doc.create(),
      sideDocId,
      dekSide.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const bridgeA = await attachEngines(server, account.deviceToken, [homeA, sideA]);
    expect(bridgeA.docIds()).toEqual([account.primaryDocId, sideDocId]);
    // "online" fires once the handshake is done and each engine has
    // sent its OpenDoc; the per-doc open + pull completes after.
    await waitFor(() => homeA.isIdle() && sideA.isIdle(), "both docs open and pulled");

    // Interleave mutations across the two docs over the one socket.
    const plan: Array<[SyncEngine, string]> = [
      [homeA, "home 0"],
      [sideA, "side 0"],
      [homeA, "home 1"],
      [sideA, "side 1"],
    ];
    const expectedSeq = new Map<SyncEngine, bigint>([
      [homeA, 0n],
      [sideA, 0n],
    ]);
    for (const [engine, text] of plan) {
      engine.addItem(LIST_MAIN, text);
      engine.captureLocalOps();
      engine.flush();
      bridgeA.pumpOutbox();
      const want = expectedSeq.get(engine)! + 1n;
      expectedSeq.set(engine, want);
      await waitFor(() => engine.lastContiguousSeq() >= want, `${text} acked`);
    }
    // Each doc counted on its own: two ops each, not four on one.
    expect(homeA.lastContiguousSeq()).toBe(2n);
    expect(sideA.lastContiguousSeq()).toBe(2n);

    // Device B bootstraps both docs over its own single socket.
    const tokenB = await registerDevice(server, account.deviceToken, "device-b");
    const homeB = new SyncEngine(
      Doc.empty(),
      account.primaryDocId,
      dekHome.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const sideB = new SyncEngine(
      Doc.empty(),
      sideDocId,
      dekSide.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const bridgeB = await attachEngines(server, tokenB, [homeB, sideB]);
    await waitFor(
      () =>
        hex(homeA.fingerprint()) === hex(homeB.fingerprint()) &&
        hex(sideA.fingerprint()) === hex(sideB.fingerprint()),
      "both docs converge",
      10_000,
    );
    expect(itemTexts(homeB)).toEqual(["home 0", "home 1"]);
    expect(itemTexts(sideB)).toEqual(["side 0", "side 1"]);

    // Live broadcast after bootstrap, in both directions, stays on
    // its own doc.
    sideB.addItem(LIST_MAIN, "side 2");
    sideB.captureLocalOps();
    sideB.flush();
    bridgeB.pumpOutbox();
    await waitFor(() => itemTexts(sideA).includes("side 2"), "side broadcast reaches A");
    homeA.addItem(LIST_MAIN, "home 2");
    homeA.captureLocalOps();
    homeA.flush();
    bridgeA.pumpOutbox();
    await waitFor(() => itemTexts(homeB).includes("home 2"), "home broadcast reaches B");
    expect(itemTexts(homeA)).toEqual(["home 0", "home 1", "home 2"]);
    expect(itemTexts(homeB)).toEqual(["home 0", "home 1", "home 2"]);
    expect(itemTexts(sideA)).toEqual(["side 0", "side 1", "side 2"]);
    expect(itemTexts(sideB)).toEqual(["side 0", "side 1", "side 2"]);

    bridgeA.stop();
    bridgeB.stop();
  }, 30_000);

  test("a doc created mid-session is routed by addEngine and reaches a second device", async () => {
    const dekHome = Dek.generate();
    const dekNew = Dek.generate();
    const account = await signup(server, dekHome.clone());
    const homeA = new SyncEngine(
      Doc.create(),
      account.primaryDocId,
      dekHome.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const bridgeA = await attachEngines(server, account.deviceToken, [homeA]);
    await waitFor(() => homeA.isIdle(), "home open and pulled");

    // The web client's create-doc flow: `POST /api/docs`, then a fresh
    // seeded engine handed to the already-open bridge.
    const newDocId = await createDoc(server, account.deviceToken);
    const newA = new SyncEngine(
      Doc.create(),
      newDocId,
      dekNew.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    bridgeA.addEngine(newA);
    expect(bridgeA.docIds()).toEqual([account.primaryDocId, newDocId]);
    expect(() => bridgeA.addEngine(newA)).toThrow();
    await waitFor(() => newA.isIdle(), "new doc opened on the live socket");
    newA.addItem(LIST_MAIN, "fresh 0");
    newA.captureLocalOps();
    newA.flush();
    bridgeA.pumpOutbox();
    await waitFor(() => newA.lastContiguousSeq() >= 1n, "fresh 0 acked");
    // Home is untouched by the new doc's traffic.
    expect(homeA.lastContiguousSeq()).toBe(0n);

    const tokenB = await registerDevice(server, account.deviceToken, "device-b");
    const newB = new SyncEngine(
      Doc.empty(),
      newDocId,
      dekNew.clone(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    );
    const bridgeB = await attachEngines(server, tokenB, [newB]);
    await waitFor(
      () => hex(newA.fingerprint()) === hex(newB.fingerprint()),
      "new doc converges on B",
      10_000,
    );
    expect(itemTexts(newB)).toEqual(["fresh 0"]);

    // The delete-doc flow's bridge half: the doc leaves the routing
    // table and its engine goes offline; Home keeps syncing.
    expect(bridgeA.removeEngine(newDocId)).toBe(true);
    expect(bridgeA.removeEngine(newDocId)).toBe(false);
    expect(bridgeA.docIds()).toEqual([account.primaryDocId]);
    homeA.addItem(LIST_MAIN, "home still live");
    homeA.captureLocalOps();
    homeA.flush();
    bridgeA.pumpOutbox();
    await waitFor(() => homeA.lastContiguousSeq() >= 1n, "home push acked after removal");

    bridgeA.stop();
    bridgeB.stop();
  }, 30_000);
});
