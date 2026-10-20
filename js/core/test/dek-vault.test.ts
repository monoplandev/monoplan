// The vault's at-rest record shape: one AES-GCM-wrapped entry per doc
// id under a single non-extractable wrapping key. `sealSession` /
// `openRecord` are the pure halves of `DekVault.save` / `load`; the IDB
// I/O around them is not exercised here (Bun has no IndexedDB).

import { describe, expect, test } from "bun:test";

import { Dek } from "../wasm/monoplan_core_web.js";
import {
  openRecord,
  sealSession,
  type VaultRecord,
  type VaultedSession,
} from "../src/storage/dek-vault.ts";

const fromHex = (hex: string) => Dek.fromHex(hex);

function session(docCount: number): VaultedSession {
  const primaryDocId = crypto.randomUUID();
  const dek = Dek.generate();
  const docKeys = new Map<string, Dek>([[primaryDocId, dek]]);
  for (let i = 1; i < docCount; i++) {
    docKeys.set(crypto.randomUUID(), Dek.generate());
  }
  return {
    anonymous: false,
    accountId: "acct-1",
    primaryDocId,
    email: "a@example.com",
    deviceId: "dev-1",
    dek,
    docKeys,
  };
}

describe("DekVault record", () => {
  test("seal then open restores every membership", async () => {
    const s = session(3);
    const rec = await sealSession(s);
    expect(Object.keys(rec.docKeys).sort()).toEqual([...s.docKeys.keys()].sort());

    const back = await openRecord(rec, fromHex);
    expect(back).not.toBeNull();
    expect(back!.primaryDocId).toBe(s.primaryDocId);
    expect(back!.docKeys.size).toBe(3);
    for (const [docId, dek] of s.docKeys) {
      expect(back!.docKeys.get(docId)!.toHex()).toBe(dek.toHex());
    }
    expect(back!.dek.toHex()).toBe(s.dek.toHex());
    expect(back!.dek).toBe(back!.docKeys.get(s.primaryDocId)!);
  });

  test("sealing does not consume the session's Dek handles", async () => {
    const s = session(2);
    await sealSession(s);
    for (const dek of s.docKeys.values()) expect(dek.toHex()).toHaveLength(64);
  });

  test("each entry gets its own IV", async () => {
    const rec = await sealSession(session(3));
    const ivs = Object.values(rec.docKeys).map((e) => Buffer.from(e.iv).toString("hex"));
    expect(new Set(ivs).size).toBe(3);
  });

  test("refuses a session whose docKeys lacks the primary", async () => {
    const s = session(2);
    s.docKeys.delete(s.primaryDocId);
    await expect(sealSession(s)).rejects.toThrow(/primary/);
  });

  test("a record with no primary entry reads as no session", async () => {
    const s = session(2);
    const rec = await sealSession(s);
    delete rec.docKeys[s.primaryDocId];
    expect(await openRecord(rec, fromHex)).toBeNull();
  });

  test("a tampered entry throws rather than yielding a bad key", async () => {
    const s = session(2);
    const rec = await sealSession(s);
    const [other] = Object.keys(rec.docKeys).filter((id) => id !== s.primaryDocId);
    rec.docKeys[other].wrappedDek[0] ^= 0xff;
    await expect(openRecord(rec, fromHex)).rejects.toThrow();
  });

  test("a pre-per-doc record (top-level iv + wrappedDek) opens as the primary alone", async () => {
    const s = session(1);
    const rec = await sealSession(s);
    const entry = rec.docKeys[s.primaryDocId];
    const legacy = {
      anonymous: true,
      accountId: rec.accountId,
      primaryDocId: rec.primaryDocId,
      email: null,
      deviceId: null,
      wrappingKey: rec.wrappingKey,
      iv: entry.iv,
      wrappedDek: entry.wrappedDek,
    } as unknown as VaultRecord;
    const back = await openRecord(legacy, fromHex);
    expect(back).not.toBeNull();
    expect(back!.anonymous).toBe(true);
    expect([...back!.docKeys.keys()]).toEqual([s.primaryDocId]);
    expect(back!.dek.toHex()).toBe(s.dek.toHex());
  });
});
