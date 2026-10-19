// Browser-specific construction of the shared `SyncBridge`. The
// pump + reconnect + auth-probe lifecycle lives in `@monoplan/core`;
// this module only contributes the things that depend on the browser
// environment: the `WebSocket` factory (cookie auth via same-origin,
// URL derived from `window.location`), the auth probe (the
// `/api/devices` endpoint via our msgpack HTTP client), the bundler
// wasm build's handshake helpers, and the client identity.

import {
  SyncBridge,
  type SyncBridgeOpts,
} from "@monoplan/core/sync-bridge";
import {
  helloFrame,
  parseHelloResponse,
  serverFrameDocId,
} from "@monoplan/core/wasm";
import { api, ApiError } from "../api.ts";

const CLIENT_NAME = "monoplan-web";
const CLIENT_VERSION = "0.1.0";

type WebOpts = Omit<
  SyncBridgeOpts,
  "socketFactory" | "probeAuth" | "codec" | "clientName" | "clientVersion"
>;

export function createSyncBridge(opts: WebOpts): SyncBridge {
  return new SyncBridge({
    ...opts,
    codec: { helloFrame, parseHelloResponse, serverFrameDocId },
    clientName: CLIENT_NAME,
    clientVersion: CLIENT_VERSION,
    socketFactory: () => new WebSocket(wsUrl()),
    probeAuth: async () => {
      try {
        await api.listDevices();
        return true;
      } catch (e) {
        // 401 from a known-authed endpoint = server says our cookie is
        // bad (revoked device, password reset elsewhere). Anything
        // else (network drop, 5xx) → reconnect.
        if (e instanceof ApiError && e.status === 401) return false;
        return true;
      }
    },
  });
}

export { SyncBridge };

function wsUrl(): string {
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${window.location.host}/api/sync`;
}
