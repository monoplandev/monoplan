// Auth form (login + signup + recovery). Argon2id currently runs on the
// main thread, so show a "deriving keys…" spinner during the
// ~hundreds-of-ms hit. Signup always enrols a 12-word recovery code,
// shown once in a pane the dialog cannot be closed around; the login form
// links to the recover mode, which redeems the code for a new password
// (spec/encryption.md §Recovery). The API origin is the page's origin (vite
// proxies /api/* in dev) — no runtime server picker; self-hosters
// serve their own bundle from their own domain.

import { Dialog } from "@kobalte/core/dialog";
import { createSignal, For, Show } from "solid-js";
import {
  Dek,
  deriveLogin,
  deriveRecovery,
  unwrapDek,
  unwrapDekWithDek,
  wrapDek,
} from "@monoplan/core/wasm";
import { api, ApiError, type LoginResponse, type Membership } from "./api.ts";
import { dekVault } from "./sync/dekVault.ts";
import { unwrapNewMemberships } from "./sync/docKeys.ts";
import {
  enrolRecovery,
  normaliseRecoveryCode,
  openRecoveryWrap,
  randomBytes,
} from "./sync/recovery.ts";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems, trackOverlay } from "./overlay.ts";
import { useSession } from "./SessionContext.tsx";

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
  /** The Home DEK: the primary doc's key, which is also the account key
   *  every other membership's wrap is under (spec/encryption.md). */
  dek: Dek;
  /** Every doc the account can open, keyed by doc id, primary included
   *  (its entry is `dek`). Populated from the memberships array at
   *  login/signup and vaulted one entry per doc id, so a session restored
   *  on reload holds every membership. `BootGate` boots a doc + engine
   *  for each entry; the UI renders the primary (sharing-plan.md Phase 5). */
  docKeys: Map<string, Dek>;
  /** True iff the Home doc does not exist yet and boot must create and
   *  seed it with the built-in lists: a freshly minted anonymous
   *  session, or a signup that could not adopt one. False for login
   *  sessions, for reload-restored sessions, and for a signup that
   *  adopted the anonymous doc — in all of those the local IndexedDB op
   *  log is the source of truth. */
  freshSignup: boolean;
}

/** The anonymous session's doc that signup carries into the account:
 *  its locally minted id becomes the server-side primary doc id and its
 *  DEK becomes the Home DEK (wrapped under the new password KEK and the
 *  recovery code). The op log and snapshot in IndexedDB are already
 *  under that id and key, so nothing is copied or re-encrypted; the
 *  engine's empty `serverKnownVv` makes the first connect push the
 *  whole history to the account's empty Home doc. */
interface AdoptedDoc {
  primaryDocId: string;
  dek: Dek;
}

/** Illustration shown as the left half of the auth dialog on wide
 *  screens; a header band above the form on narrow viewports (see
 *  `.auth-dialog-art`). */
const AUTH_ART_URL = "/monoplan-logo.png";

/** Mailing-list signup linked from the pre-release notice. */
const MAILING_LIST_URL = "https://monoplan.app/#mailing-list";

function defaultDeviceName(): string {
  return `web-${typeof navigator !== "undefined" ? navigator.platform : "unknown"}`;
}

export type AuthMode = "login" | "signup" | "recover";

/** Default Argon2id params (mirror `KdfParams::DEFAULT` in the protocol
 *  crate), used wherever this client mints fresh password material. */
const KDF_DEFAULT = { m_kib: 64 * 1024, t: 3, p: 1 };

function AuthForm(props: {
  initialMode?: AuthMode;
  onSession: (s: Session) => void;
  /** Signup: the session is live, but the dialog must show the new
   *  recovery code before handing the session over. */
  onRecoveryCode: (s: Session, code: string) => void;
}) {
  const { m } = useAppI18n();
  const { session } = useSession();
  const [mode, setMode] = createSignal<AuthMode>(props.initialMode ?? "login");
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [recoveryCode, setRecoveryCode] = createSignal("");
  const [deviceName] = createSignal(defaultDeviceName());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const switchMode = (next: AuthMode) => {
    setError(null);
    setMode(next);
  };

  const submit = async (e: Event) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      switch (mode()) {
        case "login":
          props.onSession(await doLogin(email(), password(), deviceName()));
          break;
        case "signup": {
          // Signing up from the anonymous session adopts its doc rather
          // than starting a fresh one; a signup while already authed
          // (not reachable from the UI today) would start fresh.
          const current = session();
          const adopt: AdoptedDoc | null = current.anonymous
            ? { primaryDocId: current.primaryDocId, dek: current.dek }
            : null;
          const { session: next, recoveryCode } = await doSignup(
            email(),
            password(),
            deviceName(),
            adopt,
          );
          props.onRecoveryCode(next, recoveryCode);
          break;
        }
        case "recover": {
          let code: string;
          try {
            code = normaliseRecoveryCode(recoveryCode());
          } catch {
            setError(m().auth.recoveryCodeInvalid);
            return;
          }
          props.onSession(await doRecover(email(), code, password(), deviceName()));
          break;
        }
      }
    } catch (err) {
      setError(humanError(err, m().auth.recoveryNotEnrolled));
    } finally {
      setBusy(false);
    }
  };

  const title = () =>
    mode() === "login"
      ? m().auth.signIn
      : mode() === "signup"
        ? m().auth.signUp
        : m().auth.resetPassword;

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
      <Dialog.Title class="auth-dialog-title">{title()}</Dialog.Title>
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
      <Show when={mode() === "recover"}>
        <label>
          {m().auth.recoveryCode}
          <textarea
            required
            rows={3}
            autocomplete="off"
            autocapitalize="off"
            spellcheck={false}
            value={recoveryCode()}
            disabled={busy()}
            onInput={(e) => setRecoveryCode(e.currentTarget.value)}
          />
        </label>
      </Show>
      <label>
        {mode() === "recover" ? m().auth.newPassword : m().auth.password}
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
      <Show when={mode() === "signup"}>
        <p class="auth-hint">{m().auth.signupRecoveryNote}</p>
      </Show>
      <input type="hidden" value={deviceName()} />
      <button type="submit" disabled={busy()}>
        {busy() ? m().auth.derivingKeys : title()}
      </button>
      <Show when={mode() === "login"}>
        <button
          type="button"
          class="auth-form-toggle"
          disabled={busy()}
          onClick={() => switchMode("signup")}
        >
          {m().auth.noAccount}
        </button>
        <button
          type="button"
          class="auth-form-toggle"
          disabled={busy()}
          onClick={() => switchMode("recover")}
        >
          {m().auth.forgotPassword}
        </button>
      </Show>
      <Show when={mode() !== "login"}>
        <button
          type="button"
          class="auth-form-toggle"
          disabled={busy()}
          onClick={() => switchMode("login")}
        >
          {mode() === "signup" ? m().auth.haveAccount : m().auth.backToSignIn}
        </button>
      </Show>
      <Show when={error()}>
        <div class="error">{error()}</div>
      </Show>
    </form>
  );
}

/** The one time the recovery code is ever shown. Lives inside the auth
 *  dialog in place of the form; the dialog refuses to close around it,
 *  so the only way on is to confirm the words are written down. */
function RecoveryCodePane(props: { code: string; onContinue: () => void }) {
  const { m } = useAppI18n();
  const [saved, setSaved] = createSignal(false);
  const [copied, setCopied] = createSignal(false);
  const words = () => props.code.split(" ");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(props.code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard denied (insecure context, permissions): the words are
      // on screen and selectable; nothing else to do.
    }
  };
  return (
    <div class="auth-form recovery-pane">
      <Dialog.Title class="auth-dialog-title">{m().auth.recoveryTitle}</Dialog.Title>
      <p class="auth-hint">{m().auth.recoveryIntro}</p>
      <ol class="recovery-code" aria-label={m().auth.recoveryCode}>
        <For each={words()}>{(w) => <li>{w}</li>}</For>
      </ol>
      <button type="button" class="auth-form-toggle" onClick={copy}>
        {copied() ? m().auth.copied : m().common.copy}
      </button>
      <label class="auth-check">
        <input
          type="checkbox"
          checked={saved()}
          onChange={(e) => setSaved(e.currentTarget.checked)}
        />
        <span>{m().auth.recoverySaved}</span>
      </label>
      <button type="submit" disabled={!saved()} onClick={props.onContinue}>
        {m().auth.continue}
      </button>
    </div>
  );
}

/** Controlled Kobalte Dialog wrapping the auth form. Mirrors the shape
 *  of `Settings` — App owns the open signal, the dialog owns its own
 *  portal/overlay/close-button. ESC and overlay click both close. */
export function AuthDialog(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  onSession: (s: Session) => void;
  initialMode?: AuthMode;
}) {
  const { m } = useAppI18n();
  trackOverlay(() => props.open);
  // A signup that enrolled a recovery code parks its session here until
  // the user confirms the code is saved. The account already exists on
  // the server and the vault is written, so the session must reach the
  // owner regardless; closing is simply refused in the meantime.
  const [pending, setPending] = createSignal<{ session: Session; code: string } | null>(
    null,
  );
  const onOpenChange = (open: boolean) => {
    if (!open && pending()) return;
    props.onOpenChange(open);
  };
  const finish = () => {
    const p = pending();
    if (!p) return;
    setPending(null);
    props.onSession(p.session);
  };
  return (
    <Dialog open={props.open} onOpenChange={onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content class="auth-dialog" onCloseAutoFocus={closeToItems}>
            <Show when={!pending()}>
              <Dialog.CloseButton class="auth-dialog-close" aria-label={m().common.close}>
                <CloseIcon />
              </Dialog.CloseButton>
            </Show>
            <div class="auth-dialog-art" aria-hidden="true">
              <img src={AUTH_ART_URL} alt="" decoding="async" />
            </div>
            <Show
              when={pending()}
              fallback={
                <AuthForm
                  initialMode={props.initialMode}
                  onSession={props.onSession}
                  onRecoveryCode={(session, code) => setPending({ session, code })}
                />
              }
            >
              {(p) => <RecoveryCodePane code={p().code} onContinue={finish} />}
            </Show>
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
  const docKeys = unwrapMemberships(
    derived.kek,
    resp.primary_doc_id,
    resp.memberships,
  );
  const dek = docKeys.get(resp.primary_doc_id)!;
  const session: Session = {
    anonymous: false,
    email,
    accountId: resp.account_id,
    primaryDocId: resp.primary_doc_id,
    deviceId: resp.device.device_id,
    dek,
    docKeys,
    freshSignup: false,
  };
  await persistVault(session);
  return session;
}

/** Turn a memberships array into DEKs. The primary membership's wrap is
 *  under the password KEK and yields the Home DEK; every other wrap is
 *  under the Home DEK. Same contract as the CLI's
 *  `keystore::unwrap_memberships`; also the shape a `GET /api/docs`
 *  refresh will use once the client boots every membership. Throws if
 *  the primary is missing or any wrap fails to open (wrong password, or
 *  a server handing back a wrap under a key we do not hold). */
export function unwrapMemberships(
  kek: Uint8Array,
  primaryDocId: string,
  memberships: Membership[],
): Map<string, Dek> {
  const primary = memberships.find((m) => m.doc_id === primaryDocId);
  if (!primary) {
    throw new Error("login response has no membership for the primary doc");
  }
  const home = unwrapDek(kek, primary.wrapped_dek, primary.wrapped_dek_nonce);
  const out = new Map<string, Dek>();
  for (const m of memberships) {
    out.set(
      m.doc_id,
      m.doc_id === primaryDocId
        ? home
        : unwrapDekWithDek(home, m.wrapped_dek, m.wrapped_dek_nonce),
    );
  }
  return out;
}

function missingDeviceCredentialMessage(): string {
  return "server did not return a device credential";
}

async function doSignup(
  email: string,
  password: string,
  deviceName: string,
  adopt: AdoptedDoc | null,
): Promise<{ session: Session; recoveryCode: string }> {
  const kdfParams = KDF_DEFAULT;
  const masterSalt = randomBytes(16);
  const derived = deriveLogin(
    password,
    masterSalt,
    kdfParams.m_kib,
    kdfParams.t,
    kdfParams.p,
  );
  // The Home DEK is the anonymous doc's DEK when there is one to adopt,
  // else freshly generated. Either way it is wrapped with the KEK
  // locally before shipping; it is the only key the KEK ever wraps, and
  // docs created later are wrapped under it (`wrapDekWithDek`,
  // `api.createDoc`). The server never sees the DEK, so an adopted key
  // is indistinguishable from a new one on its side.
  const dek = adopt?.dek ?? Dek.generate();
  const wrapped = wrapDek(derived.kek, dek);
  // Mandatory enrolment. The recovery wrap is of the same Home DEK, so
  // the code covers every doc the account joins later.
  const recovery = enrolRecovery(dek, kdfParams);
  const resp = await api.signup({
    email,
    master_salt: masterSalt,
    kdf_params: kdfParams,
    auth_secret: derived.authSecret,
    wrapped_dek: wrapped.ciphertext,
    wrapped_dek_nonce: wrapped.nonce,
    recovery: recovery.material,
    device_name: deviceName,
    primary_doc_id: adopt?.primaryDocId,
  });
  if (adopt && resp.primary_doc_id !== adopt.primaryDocId) {
    // The server honoured the request or rejected it (409); a silent
    // substitution would strand the local history under the old id.
    throw new Error("server did not adopt the local doc id at signup");
  }
  const session: Session = {
    anonymous: false,
    email,
    accountId: resp.account_id,
    primaryDocId: resp.primary_doc_id,
    deviceId: resp.device_id,
    dek,
    docKeys: new Map([[resp.primary_doc_id, dek]]),
    // An adopted doc already exists in IndexedDB (seeded, with the
    // user's history); boot replays it instead of creating a new one.
    freshSignup: adopt === null,
  };
  await persistVault(session);
  return { session, recoveryCode: recovery.code };
}

/** Redeem a recovery code for a new password, registering this client
 *  as a device on the way (the CLI's `monoplan auth recover`). The code
 *  opens the primary membership's wrap; every other membership is under
 *  the Home DEK and is read back from `GET /api/docs` once the reset
 *  has made us a device. Not a fresh signup: the server already holds
 *  the account's history and the bootstrap pulls it. */
async function doRecover(
  email: string,
  recoveryCode: string,
  newPassword: string,
  deviceName: string,
): Promise<Session> {
  const pre = await api.prelogin(email);
  if (!pre.recovery_salt) {
    throw new ApiError(403, "recovery_not_enrolled", "", "recovery_not_enrolled");
  }
  const recovery = deriveRecovery(
    recoveryCode,
    pre.recovery_salt,
    pre.kdf_params.m_kib,
    pre.kdf_params.t,
    pre.kdf_params.p,
  );
  const recovered = await api.recover({
    email,
    recovery_auth_secret: recovery.recoveryAuthSecret,
  });
  const dek = openRecoveryWrap(
    recovery.recoveryKek,
    recovered.primary_doc_id,
    recovered.doc_keys,
  );

  // New password material; the server replaces the primary membership's
  // password wrap and refuses anything but exactly that doc.
  const newSalt = randomBytes(16);
  const next = deriveLogin(
    newPassword,
    newSalt,
    KDF_DEFAULT.m_kib,
    KDF_DEFAULT.t,
    KDF_DEFAULT.p,
  );
  const rewrapped = wrapDek(next.kek, dek);
  const reset = await api.passwordReset({
    recovery_session_token: recovered.recovery_session_token,
    new_master_salt: newSalt,
    new_kdf_params: KDF_DEFAULT,
    new_auth_secret: next.authSecret,
    new_doc_keys: [
      {
        doc_id: recovered.primary_doc_id,
        wrapped_dek: rewrapped.ciphertext,
        wrapped_dek_nonce: rewrapped.nonce,
      },
    ],
    device_name: deviceName,
  });
  if (reset.primary_doc_id !== recovered.primary_doc_id) {
    throw new Error("server changed primary_doc_id between recover and reset");
  }

  const docs = await api.listDocs();
  const docKeys = unwrapNewMemberships(dek, reset.primary_doc_id, docs.memberships, new Set());
  docKeys.set(reset.primary_doc_id, dek);
  const session: Session = {
    anonymous: false,
    email,
    accountId: recovered.account_id,
    primaryDocId: reset.primary_doc_id,
    deviceId: reset.device_id,
    dek,
    docKeys,
    freshSignup: false,
  };
  await persistVault(session);
  return session;
}

async function persistVault(session: Session): Promise<void> {
  // Wrap every doc's DEK and stash them in IndexedDB so a reload skips
  // the password prompt. Failure is non-fatal — the in-memory session is
  // still usable; the user just gets bounced back to login on reload.
  try {
    await dekVault.save({
      anonymous: session.anonymous,
      accountId: session.accountId,
      primaryDocId: session.primaryDocId,
      email: session.email,
      deviceId: session.deviceId,
      dek: session.dek,
      docKeys: session.docKeys,
    });
  } catch (e) {
    console.warn("dekVault.save failed; session will not survive reload:", e);
  }
}

function humanError(e: unknown, notEnrolled: string): string {
  if (e instanceof ApiError && e.code === "recovery_not_enrolled") return notEnrolled;
  if (e instanceof ApiError) return `${e.code}: ${e.message}`;
  if (e instanceof Error) return e.message;
  return String(e);
}
