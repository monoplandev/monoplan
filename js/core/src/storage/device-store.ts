// Per-account device row — sync identity + the "last synced" stamp.
//
// Lives in the `device` store of the `monoplan-web` database (alongside
// vault + prefs and the engine op log), keyed per account. The resume
// cursor used to live here too, but it's now the engine's: persisted in
// the `docs` store via `IdbStorage.writeAckedSeq` (clamped to the
// durable contiguous frontier — see `spec/local-storage.md`). This row
// carries only identity and `lastSyncAt`, which is observability for the
// "Synced …" status and never consulted by the sync path.

import { type DeviceConfig, normalizeDeviceConfig } from "./adapter.ts";
import { openMonoplanDb, STORE_DEVICE } from "./web-db.ts";

interface DeviceRow {
  account_id: string;
  device: DeviceConfig;
}

export async function getDevice(
  accountId: string,
): Promise<DeviceConfig | null> {
  const db = await openMonoplanDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_DEVICE, "readonly");
    const req = tx.objectStore(STORE_DEVICE).get(accountId);
    req.onsuccess = () => {
      const row = req.result as DeviceRow | undefined;
      resolve(normalizeDeviceConfig(row?.device));
    };
    req.onerror = () => reject(req.error);
  });
}

/** Drop the per-account device row (no-op when absent). */
export async function deleteDevice(accountId: string): Promise<void> {
  const db = await openMonoplanDb();
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE_DEVICE, "readwrite");
    tx.objectStore(STORE_DEVICE).delete(accountId);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error ?? new Error("device delete aborted"));
  });
}

export async function putDevice(
  accountId: string,
  device: DeviceConfig,
): Promise<void> {
  const db = await openMonoplanDb();
  const row: DeviceRow = { account_id: accountId, device };
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE_DEVICE, "readwrite");
    tx.objectStore(STORE_DEVICE).put(row);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error ?? new Error("device put aborted"));
  });
}
