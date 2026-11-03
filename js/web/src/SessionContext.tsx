// App-level session context: the current booted session plus its live
// sync status and lifecycle actions. `App` provides it inside the keyed
// `<Show>` — so `session()` is always non-null here — and the workspace
// tree consumes it via `useSession()`. This collapses what used to drill
// through `BootGate` and `MainApp` as pure pass-through props.
//
// Accessors are kept separate (not merged into one store) so a consumer
// reading `online()` doesn't re-run when only `lastSyncAt()` changes.

import {
  createContext,
  useContext,
  type Accessor,
  type JSX,
  type Setter,
} from "solid-js";
import { type Session } from "./Login.tsx";

export interface SessionContextValue {
  /** The current booted session — never null within the provider. */
  session: Accessor<Session>;
  /** True while the WebSocket sync bridge is connected. */
  online: Accessor<boolean>;
  setOnline: Setter<boolean>;
  /** Wall-clock ms of the last successful server frame, or null. */
  lastSyncAt: Accessor<number | null>;
  setLastSyncAt: Setter<number | null>;
  /** Tear down server state and drop to a fresh anonymous session.
   *  `wipeLocal` also deletes the local copy of every doc (op log,
   *  snapshot, device row, prefs): the user-initiated sign-out, taken
   *  after a confirmation that names unsynced changes. The sync
   *  runtime's auth-failure path calls this without it, so an expired
   *  cookie never destroys data. `serverGone` skips the server call:
   *  the account was just deleted, so there is no device to revoke. */
  logout: (opts?: { wipeLocal?: boolean; serverGone?: boolean }) => void;
  /** Replace the current session (e.g. anonymous → authenticated). */
  swapSession: (s: Session) => void;
}

const SessionContext = createContext<SessionContextValue>();

export function SessionProvider(props: {
  value: SessionContextValue;
  children: JSX.Element;
}): JSX.Element {
  return (
    <SessionContext.Provider value={props.value}>
      {props.children}
    </SessionContext.Provider>
  );
}

export function useSession(): SessionContextValue {
  const ctx = useContext(SessionContext);
  if (!ctx) {
    throw new Error("useSession must be used within a SessionProvider");
  }
  return ctx;
}

/** Whether "Copy link" is offered (`spec/urls.md` "Affordances"): on any
 *  session with an account behind it, online or not, since the link
 *  names the server's web client and stays valid. An accountless
 *  (anonymous) session has nowhere a link would resolve for anyone else,
 *  so the item is disabled. The macOS app's window is always signed in.
 *  Returns a plain function so a consumer tracks it where it reads it. */
export function useCanCopyLink(): Accessor<boolean> {
  const { session } = useSession();
  return () => !session().anonymous;
}
