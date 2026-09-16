// Auth form (login + signup). Argon2id currently runs on the main
// thread, so show a "deriving keys…" spinner during the
// ~hundreds-of-ms hit. The API origin is the page's origin (vite
// proxies /api/* in dev) — no runtime server picker; self-hosters
// serve their own bundle from their own domain.

import { Dialog } from "@kobalte/core/dialog";
import { createSignal, Show } from "solid-js";
import {
  Dek,
  deriveLogin,
  unwrapDek,
  wrapDek,
} from "@monoplan/core/wasm";
import { api, ApiError, type LoginResponse } from "./api.ts";
import { dekVault } from "./sync/dekVault.ts";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems, trackOverlay } from "./overlay.ts";

export interface Session {
  /** Local-only session with no server account behind it. The web client
   *  drops a freshly-generated DEK into the vault on first visit so the
   *  user can use the app without an auth wall; sync stays off until they
   *  sign up or log in. */
  anonymous: boolean;
  /** Account id — server-issued for authenticated sessions, locally
   *  generated (`anon-<uuid>`) for anonymous ones. Used to namespace
   *  per-account state (device row, prefs). */
  accountId: string;
  /** Doc id of the account's primary (Home) doc — server-assigned for
   *  authenticated sessions, locally generated for anonymous ones. Keys
   *  the per-doc data plane (the engine op log + snapshot in IndexedDB). */
  primaryDocId: string;
  /** Null on anonymous sessions. */
  email: string | null;
  /** Null on anonymous sessions. */
  deviceId: string | null;
  dek: Dek;
  /** True iff this session was created via signup (we're device 1
   *  and need to seed the doc with built-in lists), or for a freshly
   *  minted anonymous session. False for login sessions and for
   *  reload-restored sessions where the local IndexedDB op log is the
   *  source of truth. */
  freshSignup: boolean;
}

/** Illustration shown as the left half of the auth dialog on wide
 *  screens; a header band above the form on narrow viewports (see
 *  `.auth-dialog-art`). */
const AUTH_ART_URL = "/monoplan-badge.png";

/** Mailing-list signup linked from the pre-release notice. */
const MAILING_LIST_URL = "https://monoplan.app/#mailing-list";

function defaultDeviceName(): string {
  return `web-${typeof navigator !== "undefined" ? navigator.platform : "unknown"}`;
}

function AuthForm(props: {
  initialMode?: "login" | "signup";
  onSession: (s: Session) => void;
}) {
  const { m } = useAppI18n();
  const [mode, setMode] = createSignal<"login" | "signup">(
    props.initialMode ?? "login",
  );
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [deviceName] = createSignal(defaultDeviceName());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const submit = async (e: Event) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const session =
        mode() === "login"
          ? await doLogin(email(), password(), deviceName())
          : await doSignup(email(), password(), deviceName());
      props.onSession(session);
    } catch (err) {
      setError(humanError(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form class="auth-form" onSubmit={submit}>
      <p class="auth-prerelease-notice" role="note">
        <WarningIcon />
        <span>
          {m().auth.prereleaseNotice}{" "}
          <a href={MAILING_LIST_URL} target="_blank" rel="noopener noreferrer">
            {m().auth.prereleaseLink}
          </a>
        </span>
      </p>
      <Dialog.Title class="auth-dialog-title">
        {mode() === "login" ? m().auth.signIn : m().auth.signUp}
      </Dialog.Title>
      <label>
        {m().auth.email}
        <input
          type="email"
          required
          autocomplete="email"
          value={email()}
          disabled={busy()}
          onInput={(e) => setEmail(e.currentTarget.value)}
        />
      </label>
      <label>
        {m().auth.password}
        <input
          type="password"
          required
          minLength={10}
          autocomplete={mode() === "login" ? "current-password" : "new-password"}
          value={password()}
          disabled={busy()}
          onInput={(e) => setPassword(e.currentTarget.value)}
        />
      </label>
      <input type="hidden" value={deviceName()} />
      <button type="submit" disabled={busy()}>
        {busy()
          ? m().auth.derivingKeys
          : mode() === "login"
            ? m().auth.signIn
            : m().auth.signUp}
      </button>
      <button
        type="button"
        class="auth-form-toggle"
        disabled={busy()}
        onClick={() => setMode(mode() === "login" ? "signup" : "login")}
      >
        {mode() === "login"
          ? m().auth.noAccount
          : m().auth.haveAccount}
      </button>
      <Show when={error()}>
        <div class="error">{error()}</div>
      </Show>
    </form>
  );
}

/** Controlled Kobalte Dialog wrapping the auth form. Mirrors the shape
 *  of `Settings` — App owns the open signal, the dialog owns its own
 *  portal/overlay/close-button. ESC and overlay click both close. */
export function AuthDialog(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  onSession: (s: Session) => void;
  initialMode?: "login" | "signup";
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content class="auth-dialog" onCloseAutoFocus={closeToItems}>
            <Dialog.CloseButton class="auth-dialog-close" aria-label={m().common.close}>
              <CloseIcon />
            </Dialog.CloseButton>
            <div class="auth-dialog-art" aria-hidden="true">
              <img src={AUTH_ART_URL} alt="" decoding="async" />
            </div>
            <AuthForm
              initialMode={props.initialMode}
              onSession={props.onSession}
            />
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
}

function WarningIcon() {
  return (
    <svg
      class="auth-prerelease-icon"
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
      <line x1="12" y1="9" x2="12" y2="13" />
      <line x1="12" y1="17" x2="12.01" y2="17" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  );
}

async function doLogin(
  email: string,
  password: string,
  deviceName: string,
): Promise<Session> {
  const pre = await api.prelogin(email);
  const derived = deriveLogin(
    password,
    pre.master_salt,
    pre.kdf_params.m_kib,
    pre.kdf_params.t,
    pre.kdf_params.p,
  );
  const resp: LoginResponse = await api.login({
    email,
    auth_secret: derived.authSecret,
    device_name: deviceName,
  });
  if (!resp.device) {
    throw new Error(missingDeviceCredentialMessage());
  }
  const dek = unwrapDek(derived.kek, resp.wrapped_dek, resp.wrapped_dek_nonce);
  const session: Session = {
    anonymous: false,
    email,
    accountId: resp.account_id,
    primaryDocId: resp.primary_doc_id,
    deviceId: resp.device.device_id,
    dek,
    freshSignup: false,
  };
  await persistVault(session);
  return session;
}

function missingDeviceCredentialMessage(): string {
  return "server did not return a device credential";
}

async function doSignup(
  email: string,
  password: string,
  deviceName: string,
): Promise<Session> {
  // Default Argon2id params (mirror `KdfParams::DEFAULT` in the
  // protocol crate) plus a fresh random 16-byte salt.
  const kdfParams = { m_kib: 64 * 1024, t: 3, p: 1 };
  const masterSalt = randomBytes(16);
  const derived = deriveLogin(
    password,
    masterSalt,
    kdfParams.m_kib,
    kdfParams.t,
    kdfParams.p,
  );
  // Generate fresh DEK, wrap with KEK locally before shipping.
  const dek = Dek.generate();
  const wrapped = wrapDek(derived.kek, dek);
  const resp = await api.signup({
    email,
    master_salt: masterSalt,
    kdf_params: kdfParams,
    auth_secret: derived.authSecret,
    wrapped_dek: wrapped.ciphertext,
    wrapped_dek_nonce: wrapped.nonce,
    device_name: deviceName,
  });
  const session: Session = {
    anonymous: false,
    email,
    accountId: resp.account_id,
    primaryDocId: resp.primary_doc_id,
    deviceId: resp.device_id,
    dek,
    freshSignup: true,
  };
  await persistVault(session);
  return session;
}

async function persistVault(session: Session): Promise<void> {
  // Wrap the DEK and stash it in IndexedDB so a reload skips the
  // password prompt. Failure is non-fatal — the in-memory session is
  // still usable; the user just gets bounced back to login on reload.
  try {
    await dekVault.save({
      anonymous: session.anonymous,
      accountId: session.accountId,
      primaryDocId: session.primaryDocId,
      email: session.email,
      deviceId: session.deviceId,
      dek: session.dek.clone(),
    });
  } catch (e) {
    console.warn("dekVault.save failed; session will not survive reload:", e);
  }
}

function randomBytes(n: number): Uint8Array {
  const out = new Uint8Array(n);
  crypto.getRandomValues(out);
  return out;
}

function humanError(e: unknown): string {
  if (e instanceof ApiError) return `${e.code}: ${e.message}`;
  if (e instanceof Error) return e.message;
  return String(e);
}
