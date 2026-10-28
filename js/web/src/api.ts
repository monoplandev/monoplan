// Slim JSON-over-fetch HTTP client for the auth surface. Same wire
// format as the CLI (spec/auth.md §Wire encoding); relative paths because
// the bundle is always served from the same origin as the API (vite proxy
// in dev, single host in prod). The device token rides as an HttpOnly
// cookie set by the server — never touched from JS — so this module
// deliberately knows nothing about it on the read side.
//
// Byte fields (salts, nonces, wrapped keys) are `Uint8Array` on this side
// of the module and standard base64 strings on the wire. The conversion
// happens once, in `encodeBody` / `decodeBody`, keyed on field name: every
// byte-valued field the API speaks is listed in `BYTE_FIELDS`, so adding
// one to the protocol means adding it here too (the Rust side marks the
// same fields with `serde_b64`).

export const JSON_CT = "application/json";
export const REQUEST_ID_HEADER = "X-Request-Id";

const BYTE_FIELDS: ReadonlySet<string> = new Set([
  "master_salt",
  "new_master_salt",
  "auth_secret",
  "new_auth_secret",
  "current_auth_secret",
  "recovery_salt",
  "recovery_auth_secret",
  "recovery_wrapped_dek",
  "recovery_wrapped_dek_nonce",
  "wrapped_dek",
  "wrapped_dek_nonce",
]);

export function bytesToBase64(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** JSON-encode a request body, base64-ing every `Uint8Array` wherever
 *  it sits (the replacer sees nested objects and arrays too). */
export function encodeBody(body: unknown): string {
  return JSON.stringify(body, (_key, value: unknown) =>
    value instanceof Uint8Array ? bytesToBase64(value) : value,
  );
}

/** Parse a response body, turning every `BYTE_FIELDS` string back into a
 *  `Uint8Array`. `null` stays `null` (optional fields like `recovery_salt`). */
export function decodeBody<T>(text: string): T {
  return JSON.parse(text, (key: string, value: unknown) =>
    BYTE_FIELDS.has(key) && typeof value === "string" ? base64ToBytes(value) : value,
  ) as T;
}

export interface KdfParams {
  m_kib: number;
  t: number;
  p: number;
}

export interface PreloginResponse {
  master_salt: Uint8Array;
  kdf_params: KdfParams;
  recovery_salt?: Uint8Array | null;
}

export interface DeviceCredential {
  device_id: string;
  /** Present on the wire for CLI parity; the web client ignores this
   *  field and relies on the `monoplan_device` cookie instead. */
  device_token: string;
}

/** One doc the account can open, with the doc's DEK wrapped under a key
 *  the account holds. Which key depends on the doc: for the primary doc
 *  (`doc_id === primary_doc_id`) the wrap is the Home DEK under the
 *  password KEK (`unwrapDek`); for every other doc it is that doc's DEK
 *  under the Home DEK (`unwrapDekWithDek`). See spec/encryption.md
 *  §Wrap-states. Login and `GET /api/docs` return one per membership. */
export interface Membership {
  doc_id: string;
  role: "owner" | "member";
  wrapped_dek: Uint8Array;
  wrapped_dek_nonce: Uint8Array;
}

/** A doc's DEK under one key the recipient holds. On the recovery path
 *  it is the primary doc's DEK (the Home DEK) under the recovery KEK
 *  (`/api/account/recover`) or under the new password KEK
 *  (`/api/account/password/reset`). */
export interface DocKey {
  doc_id: string;
  wrapped_dek: Uint8Array;
  wrapped_dek_nonce: Uint8Array;
}

/** Optional signup material enrolling a recovery code: the salt the
 *  code is stretched with, the auth secret the server hashes to gate
 *  `/recover`, and the Home DEK under the recovery KEK
 *  (spec/encryption.md §Recovery). */
export interface RecoveryMaterial {
  recovery_salt: Uint8Array;
  recovery_auth_secret: Uint8Array;
  recovery_wrapped_dek: Uint8Array;
  recovery_wrapped_dek_nonce: Uint8Array;
}

export interface RecoverResponse {
  account_id: string;
  primary_doc_id: string;
  /** Exactly one entry: the primary membership's recovery wrap. */
  doc_keys: DocKey[];
  /** Single use, 15-minute TTL; spent by `/password/reset`. */
  recovery_session_token: string;
}

export interface PasswordResetResponse {
  device_id: string;
  /** CLI parity; the web client relies on the cookie set alongside. */
  device_token: string;
  primary_doc_id: string;
}

/** `GET /api/docs`: the same shape as the membership part of login, for a
 *  device that wants to learn about docs created since it logged in. */
export interface DocsListResponse {
  primary_doc_id: string;
  memberships: Membership[];
}

export interface LoginResponse {
  account_id: string;
  /** Server-assigned id of the account's primary (Home) doc. Used to key
   *  local storage so ops/snapshots live under the real doc id rather
   *  than a per-account placeholder. Always present in `memberships`. */
  primary_doc_id: string;
  memberships: Membership[];
  recovery_present: boolean;
  device?: DeviceCredential;
}

export interface Device {
  id: string;
  name: string;
  /** `web` for this client; native apps report their host platform
   *  (`macos`, `linux`, `windows`, `ios`, `android`). */
  kind: string;
  last_seen_at: number;
  last_seen_ip: string | null;
  created_at: number;
}

export interface DevicesListResponse {
  devices: Device[];
}

/** One sharing link as the owner sees it (`GET /api/docs/:id/invites`).
 *  The server holds no secret, so a listed link cannot be rebuilt, only
 *  revoked. `consumed_at` is set once accepted (single use). */
export interface Invite {
  invite_id: string;
  created_at: number;
  expires_at: number;
  consumed_at: number | null;
}

/** `GET /api/invites/:id`: what a link holder learns before signing in.
 *  The wrap is the doc's DEK under the invite KEK (`InviteSecret.unwrap`). */
export interface InvitePayload {
  doc_id: string;
  wrapped_dek: Uint8Array;
  wrapped_dek_nonce: Uint8Array;
  expires_at: number;
  /** The inviter's email, masked server-side (`d…@example.com`). */
  inviter_hint: string;
}

export interface Member {
  account_id: string;
  email: string;
  role: "owner" | "member";
  added_at: number;
}

export interface ApiErrorBody {
  code: string;
  message: string;
}

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    public readonly requestId: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/** What this client registers as (`Device.kind`). The installed PWA is
 *  still the browser client, so it is `web` too. */
const DEVICE_KIND = "web";

export const api = {
  async prelogin(email: string): Promise<PreloginResponse> {
    return post("/api/account/prelogin", { email });
  },

  async login(args: {
    email: string;
    auth_secret: Uint8Array;
    device_name: string;
  }): Promise<LoginResponse> {
    return post("/api/account/login", {
      email: args.email,
      auth_secret: args.auth_secret,
      register_device: { name: args.device_name, kind: DEVICE_KIND },
    });
  },

  async signup(args: {
    email: string;
    master_salt: Uint8Array;
    kdf_params: KdfParams;
    auth_secret: Uint8Array;
    /** The new primary doc's DEK under the password KEK; the server
     *  stores it on the account's owner membership of that doc. */
    wrapped_dek: Uint8Array;
    wrapped_dek_nonce: Uint8Array;
    /** Mandatory: every account enrols a recovery code at signup. */
    recovery: RecoveryMaterial;
    device_name: string;
    /** Adopt this id for the primary doc instead of letting the server
     *  mint one: signup from an anonymous session keeps its local doc
     *  (history, DEK and all) and ships it on first sync. 409 if a doc
     *  with that id already exists. */
    primary_doc_id?: string;
  }): Promise<{
    account_id: string;
    device_id: string;
    device_token: string;
    /** Id of the new account's primary doc — the adopted one when the
     *  request named it, else server-assigned; see LoginResponse. */
    primary_doc_id: string;
  }> {
    return post("/api/account/signup", {
      email: args.email,
      master_salt: args.master_salt,
      kdf_params: args.kdf_params,
      auth_secret: args.auth_secret,
      wrapped_dek: args.wrapped_dek,
      wrapped_dek_nonce: args.wrapped_dek_nonce,
      recovery: args.recovery,
      device_name: args.device_name,
      device_kind: DEVICE_KIND,
      primary_doc_id: args.primary_doc_id,
    });
  },

  /** Prove possession of the recovery code. 403 `recovery_not_enrolled`
   *  when the account has no code; 401 on a wrong one. Rate-limited
   *  hardest of the credential endpoints. */
  async recover(args: {
    email: string;
    recovery_auth_secret: Uint8Array;
  }): Promise<RecoverResponse> {
    return post("/api/account/recover", {
      email: args.email,
      recovery_auth_secret: args.recovery_auth_secret,
    });
  },

  /** Set a new password with the session from `recover`; also registers
   *  this client as a device and sets the device cookie. `new_doc_keys`
   *  must be exactly the primary doc's Home DEK under the new KEK. */
  async passwordReset(args: {
    recovery_session_token: string;
    new_master_salt: Uint8Array;
    new_kdf_params: KdfParams;
    new_auth_secret: Uint8Array;
    new_doc_keys: DocKey[];
    device_name: string;
  }): Promise<PasswordResetResponse> {
    return post("/api/account/password/reset", {
      recovery_session_token: args.recovery_session_token,
      new_master_salt: args.new_master_salt,
      new_kdf_params: args.new_kdf_params,
      new_auth_secret: args.new_auth_secret,
      new_doc_keys: args.new_doc_keys,
      device_name: args.device_name,
      device_kind: DEVICE_KIND,
    });
  },

  /** Server revokes the calling device's token + emits a clear-cookie. */
  async logout(): Promise<void> {
    await post<unknown>("/api/account/logout", {});
  },

  async listDevices(): Promise<DevicesListResponse> {
    return get("/api/devices");
  },

  async listDocs(): Promise<DocsListResponse> {
    return get("/api/docs");
  },

  /** Create a doc the account owns. `wrapped_dek` is the new doc's DEK
   *  under the **Home DEK** (`wrapDekWithDek`), never under the password
   *  KEK: password change / reset / recovery only ever re-wrap the Home
   *  DEK, so this wrap needs no maintenance. */
  async createDoc(args: {
    wrapped_dek: Uint8Array;
    wrapped_dek_nonce: Uint8Array;
  }): Promise<{ doc_id: string }> {
    return post("/api/docs", {
      wrapped_dek: args.wrapped_dek,
      wrapped_dek_nonce: args.wrapped_dek_nonce,
    });
  },

  /** Delete a doc the account owns (`DELETE /api/docs/:id`). The server
   *  refuses the primary doc (403), a doc the caller does not own (404)
   *  and one with other active members (409); every row under it goes
   *  with it. The caller drops its local copy and key afterwards. */
  async deleteDoc(docId: string): Promise<void> {
    await del<unknown>(`/api/docs/${encodeURIComponent(docId)}`);
  },

  // ---- sharing links (`spec/sharing-plan.md` §Invites) ----

  /** Issue a link for a doc the account owns. `invite_id` is the HKDF id
   *  of the fragment secret (`InviteSecret.id()`), `wrapped_dek` the doc's
   *  DEK under the invite KEK (`InviteSecret.wrap`). The server refuses the
   *  primary doc (403) and a doc the caller does not own (404). */
  async createInvite(
    docId: string,
    args: {
      invite_id: string;
      wrapped_dek: Uint8Array;
      wrapped_dek_nonce: Uint8Array;
      expires_in_secs?: number;
    },
  ): Promise<{ invite_id: string; expires_at: number }> {
    return post(`/api/docs/${encodeURIComponent(docId)}/invites`, {
      invite_id: args.invite_id,
      wrapped_dek: args.wrapped_dek,
      wrapped_dek_nonce: args.wrapped_dek_nonce,
      expires_in_secs: args.expires_in_secs ?? null,
    });
  },

  /** Every link issued on a doc the account owns, newest first. */
  async listInvites(docId: string): Promise<{ invites: Invite[] }> {
    return get(`/api/docs/${encodeURIComponent(docId)}/invites`);
  },

  async revokeInvite(docId: string, inviteId: string): Promise<void> {
    await del<unknown>(
      `/api/docs/${encodeURIComponent(docId)}/invites/${encodeURIComponent(inviteId)}`,
    );
  },

  /** Unauthenticated: the id is the credential. 404 for an unknown or
   *  revoked link, 410 (`gone`) once expired or used. */
  async fetchInvite(inviteId: string): Promise<InvitePayload> {
    return get(`/api/invites/${encodeURIComponent(inviteId)}`);
  },

  /** Join the doc behind a link. `wrapped_dek` is the recovered DEK under
   *  the acceptor's Home DEK (`wrapDekWithDek`), the new membership's wrap.
   *  409 (`conflict`) when the caller is already a member; the link stays
   *  open in that case. */
  async acceptInvite(
    inviteId: string,
    args: { wrapped_dek: Uint8Array; wrapped_dek_nonce: Uint8Array },
  ): Promise<{ doc_id: string }> {
    return post(`/api/invites/${encodeURIComponent(inviteId)}/accept`, {
      wrapped_dek: args.wrapped_dek,
      wrapped_dek_nonce: args.wrapped_dek_nonce,
    });
  },

  /** Active members of a doc the account is a member of, oldest first. */
  async listMembers(docId: string): Promise<{ members: Member[] }> {
    return get(`/api/docs/${encodeURIComponent(docId)}/members`);
  },

  /** Owner removes a member. 403 for a non-owner member, 404 for a
   *  non-member caller or a target who is not a member, 409 for
   *  oneself or the last owner. Forward-only: the removed account
   *  keeps what it already synced. Their live connections get
   *  `DocClosed` and drop the doc. */
  async removeMember(docId: string, accountId: string): Promise<void> {
    await del<unknown>(
      `/api/docs/${encodeURIComponent(docId)}/members/${encodeURIComponent(accountId)}`,
    );
  },

  async renameDevice(deviceId: string, name: string): Promise<void> {
    await patch<unknown>(`/api/devices/${encodeURIComponent(deviceId)}`, { name });
  },

  async deleteDevice(deviceId: string): Promise<void> {
    await del<unknown>(`/api/devices/${encodeURIComponent(deviceId)}`);
  },
};

async function post<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, { method: "POST", body: encodeBody(body) });
}

async function get<T>(path: string): Promise<T> {
  return request<T>(path, { method: "GET" });
}

async function patch<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, { method: "PATCH", body: encodeBody(body) });
}

async function del<T>(path: string): Promise<T> {
  return request<T>(path, { method: "DELETE" });
}

async function request<T>(
  path: string,
  init: { method: string; body?: string },
): Promise<T> {
  const requestId = newRequestId();
  const res = await fetch(path, {
    method: init.method,
    // Default for same-origin is already 'same-origin', but be explicit:
    // the cookie carrying the device token is the auth here, and an
    // accidental `'omit'` would silently break logged-in calls.
    credentials: "same-origin",
    headers: {
      ...(init.body !== undefined ? { "Content-Type": JSON_CT } : {}),
      Accept: JSON_CT,
      [REQUEST_ID_HEADER]: requestId,
    },
    body: init.body,
  });
  const echoedRequestId = res.headers.get(REQUEST_ID_HEADER) ?? requestId;
  const text = await res.text();
  if (!res.ok) {
    let err: ApiErrorBody | null = null;
    try {
      err = JSON.parse(text) as ApiErrorBody;
    } catch {
      // body wasn't JSON — surface a generic error
    }
    throw new ApiError(
      res.status,
      err?.code ?? "http_error",
      echoedRequestId,
      err?.message ?? `${res.status} ${res.statusText}`,
    );
  }
  if (text.length === 0) return undefined as T;
  return decodeBody<T>(text);
}

function newRequestId(): string {
  return crypto.randomUUID();
}
