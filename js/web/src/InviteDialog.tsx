import { Dialog } from "@kobalte/core/dialog";
import { Match, Show, Switch, createSignal, onMount } from "solid-js";
import type { Dek } from "@monoplan/core/wasm";
import { ApiError } from "./api.ts";
import { useAppI18n } from "./i18n.tsx";
import {
  acceptInvite,
  clearPendingInvite,
  lookupInvite,
  pendingInvite,
  type InviteLookup,
} from "./invite.ts";
import { AuthDialog } from "./Login.tsx";
import { closeToItems, trackOverlay } from "./overlay.ts";
import { useSession } from "./SessionContext.tsx";

type Stage =
  | { kind: "loading" }
  | { kind: "ready"; lookup: InviteLookup }
  | { kind: "joining"; lookup: InviteLookup }
  | { kind: "error"; message: "invalid" | "gone" | "failed" };

/** The receiving end of a sharing link (`spec/sharing-plan.md`
 *  §Invites). Mounts with the workspace; opens iff a secret was parked by
 *  `captureInviteFromLocation`, resolves it to the wrap behind it and
 *  offers to join. An anonymous session is sent through the usual
 *  sign-in / sign-up dialog first: the parked secret survives the
 *  session swap (the whole tree remounts) and this dialog comes back
 *  authenticated. "Not now" forgets the link for this tab; the link
 *  itself stays valid until it expires or is used. */
export function InviteDialog(props: {
  joinDoc: (docId: string, dek: Dek) => Promise<string>;
  onJoined: (docId: string) => void;
}) {
  const { m } = useAppI18n();
  const session = useSession();
  const [open, setOpen] = createSignal(pendingInvite() !== null);
  const [stage, setStage] = createSignal<Stage>({ kind: "loading" });
  const [authOpen, setAuthOpen] = createSignal(false);
  trackOverlay(() => open());

  onMount(async () => {
    const secret = pendingInvite();
    if (secret === null) return;
    try {
      setStage({ kind: "ready", lookup: await lookupInvite(secret) });
    } catch (e) {
      console.warn("invite lookup failed:", e);
      setStage({ kind: "error", message: errorKind(e) });
    }
  });

  const dismiss = (): void => {
    clearPendingInvite();
    setOpen(false);
  };

  const join = async (): Promise<void> => {
    const s = stage();
    if (s.kind !== "ready" || session.session().anonymous) return;
    setStage({ kind: "joining", lookup: s.lookup });
    try {
      const { docId, dek } = await acceptInvite(session.session(), s.lookup);
      await props.joinDoc(docId, dek);
      clearPendingInvite();
      setOpen(false);
      props.onJoined(docId);
    } catch (e) {
      console.error("invite accept failed:", e);
      setStage({ kind: "error", message: errorKind(e) });
    }
  };

  const anonymous = (): boolean => session.session().anonymous;

  return (
    <>
      <Dialog open={open()} onOpenChange={(b) => !b && dismiss()} modal>
        <Dialog.Portal>
          <Dialog.Overlay class="dialog-overlay" />
          <div class="dialog-positioner">
            <Dialog.Content class="confirm-dialog" onCloseAutoFocus={closeToItems}>
              <section class="confirm-dialog-section confirm-dialog-header">
                <Dialog.Title class="confirm-dialog-title">
                  {m().sharing.inviteTitle}
                </Dialog.Title>
                <Dialog.Description class="confirm-dialog-message">
                  <Switch>
                    <Match when={stage().kind === "loading"}>{m().common.loading}</Match>
                    <Match when={stageError()}>
                      {(err) => m().sharing.inviteError[err()]}
                    </Match>
                    <Match when={stageLookup()}>
                      {(lookup) => (
                        <>
                          {m().sharing.inviteFrom(lookup().payload.inviter_hint)}
                          <Show when={anonymous()}>
                            {" "}
                            {m().sharing.inviteSignIn}
                          </Show>
                        </>
                      )}
                    </Match>
                  </Switch>
                </Dialog.Description>
              </section>
              <section class="confirm-dialog-section confirm-dialog-actions">
                <button
                  type="button"
                  class="confirm-dialog-btn confirm-dialog-cancel"
                  onClick={dismiss}
                >
                  {stage().kind === "error" ? m().common.close : m().sharing.notNow}
                </button>
                <Show when={stage().kind === "ready" || stage().kind === "joining"}>
                  <Show
                    when={!anonymous()}
                    fallback={
                      <button
                        type="button"
                        class="confirm-dialog-btn confirm-dialog-confirm"
                        onClick={() => setAuthOpen(true)}
                      >
                        {m().sharing.signIn}
                      </button>
                    }
                  >
                    <button
                      type="button"
                      class="confirm-dialog-btn confirm-dialog-confirm"
                      disabled={stage().kind === "joining"}
                      onClick={() => void join()}
                    >
                      {stage().kind === "joining" ? m().sharing.joining : m().sharing.join}
                    </button>
                  </Show>
                </Show>
              </section>
            </Dialog.Content>
          </div>
        </Dialog.Portal>
      </Dialog>
      <Show when={anonymous()}>
        <AuthDialog
          open={authOpen()}
          onOpenChange={setAuthOpen}
          onSession={(s) => {
            setAuthOpen(false);
            // The tree remounts on the new session; the parked secret
            // brings this dialog back, now able to join.
            session.swapSession(s);
          }}
        />
      </Show>
    </>
  );

  function stageError(): "invalid" | "gone" | "failed" | undefined {
    const s = stage();
    return s.kind === "error" ? s.message : undefined;
  }

  function stageLookup(): InviteLookup | undefined {
    const s = stage();
    return s.kind === "ready" || s.kind === "joining" ? s.lookup : undefined;
  }
}

function errorKind(e: unknown): "invalid" | "gone" | "failed" {
  if (e instanceof ApiError) {
    if (e.status === 404) return "invalid";
    if (e.status === 410) return "gone";
  }
  return "failed";
}
