// Per-account device identity (`DeviceConfig`) plus its normalizer.
//
// This is the sync identity row — account/device ids, server URL, and
// the observability-only "last synced" stamp. On web it's persisted in
// the `device` store of the `monoplan-web` IDB via `device-store.ts`,
// alongside the replica's op log (`IdbStorage`) — one database, separate
// stores. The DEK is never stored here — its browser lifetime is
// in-memory only (see `DekVault`).

export interface DeviceConfig {
  accountId: string;
  /** Server-assigned id of the account's primary (Home) doc. */
  primaryDocId: string;
  email: string;
  serverUrl: string;
  deviceId: string;
  /** Unix millis of the last successful online sync, or null.
   *  Observability only (the "Synced …" status) — the resume cursor is
   *  the replica's, persisted via `IdbStorage.writeAckedSeq`. */
  lastSyncAt: number | null;
}

export function normalizeDeviceConfig(value: unknown): DeviceConfig | null {
  if (!value || typeof value !== "object") return null;
  const device = value as DeviceConfig;
  if (
    typeof device.accountId !== "string" ||
    typeof device.primaryDocId !== "string" ||
    typeof device.email !== "string" ||
    typeof device.serverUrl !== "string" ||
    typeof device.deviceId !== "string"
  ) {
    return null;
  }
  const lastSyncAt =
    typeof device.lastSyncAt === "number" || device.lastSyncAt === null
      ? device.lastSyncAt
      : null;
  return {
    accountId: device.accountId,
    primaryDocId: device.primaryDocId,
    email: device.email,
    serverUrl: device.serverUrl,
    deviceId: device.deviceId,
    lastSyncAt,
  };
}
