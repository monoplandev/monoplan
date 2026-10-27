// @monoplan/core — storage adapters + types. The wasm-bindgen surface
// (`Doc`, `Dek`, `SyncEngine`, ...) lives at `@monoplan/core/wasm` so
// the conditional export resolves to the bundler-target build under
// Vite and the nodejs-target build under Bun without callers
// thinking about it.

export type { DeviceConfig } from "./storage/adapter.ts";
export {
  IdbStorage,
  type EngineBootRows,
  type InFlightPushJs,
} from "./storage/idb-storage.ts";
export { openMonoplanDb, _resetMonoplanDbForTests } from "./storage/web-db.ts";
export { readOrMintPeerSlot } from "./storage/peer-slots.ts";
export { deleteDevice, getDevice, putDevice } from "./storage/device-store.ts";
export {
  DekVault,
  type DekFromHex,
  type VaultedSession,
} from "./storage/dek-vault.ts";
export {
  SyncBridge,
  type ConnectionEvent,
  type HandshakeCodec,
  type ReconnectBackoff,
  type SyncBridgeOpts,
} from "./sync-bridge.ts";
