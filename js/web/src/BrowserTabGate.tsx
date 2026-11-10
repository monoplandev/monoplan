// Single-tab gate + slot-0 peer anchor. Monoplan assumes one active tab
// per browser (the replica + IDB op log aren't built for concurrent
// writers), so we grab a Web Lock on mount and only render `App` once
// the outcome is known. In prod a second tab fails to acquire the lock
// and shows an "already open elsewhere" notice; in dev (or with
// `VITE_ENFORCE_SINGLE_TAB=0`) it renders anyway. The lock releases on
// cleanup, so closing the holding tab lets a waiting one take over on
// its next mount.
//
// The same lock acquisition anchors the stable Loro peer id
// (`spec/peer-id-plan.md`): holding `monoplan-single-tab` is the claim
// on peer slot 0, so `lockHeld` flows into boot and decides between
// `Doc.createWithPeer` (slot peer) and `Doc.create` (random peer).
// This coupling is deliberate — a tab that renders without the lock
// (dev second tab, no `navigator.locks`) MUST mint under a random
// peer, or two live docs share a peer and corrupt the CRDT.

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { useAppI18n } from "./i18n.tsx";
import { App } from "./App.tsx";

const SINGLE_TAB_LOCK_NAME = "monoplan-single-tab";

export function BrowserTabGate() {
  const { m } = useAppI18n();
  const [gate, setGate] = createSignal<"checking" | "allowed" | "blocked">(
    "checking",
  );
  const [lockHeld, setLockHeld] = createSignal(false);

  onMount(() => {
    const enforce = shouldEnforceSingleTab();
    if (!("locks" in navigator) || !navigator.locks) {
      if (enforce) {
        console.warn("navigator.locks unavailable; single-tab gate disabled");
      }
      setGate("allowed");
      return;
    }

    let release: (() => void) | null = null;
    void navigator.locks.request(
      SINGLE_TAB_LOCK_NAME,
      { ifAvailable: true },
      async (lock) => {
        if (!lock) {
          setGate(enforce ? "blocked" : "allowed");
          return;
        }
        setLockHeld(true);
        setGate("allowed");
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      },
    );

    onCleanup(() => {
      release?.();
    });
  });

  return (
    <Show when={gate() !== "checking"} fallback={<div class="empty">{m().common.loading}</div>}>
      <Show
        when={gate() === "allowed"}
        fallback={
          <div class="empty">
            Monoplan is already open in another tab.
          </div>
        }
      >
        <App singleTabLockHeld={lockHeld()} />
      </Show>
    </Show>
  );
}

function shouldEnforceSingleTab(): boolean {
  const flag = (import.meta.env as Record<string, string | boolean | undefined>)[
    "VITE_ENFORCE_SINGLE_TAB"
  ];
  if (flag === "0") return false;
  if (flag === "1") return true;

  return !import.meta.env.DEV;
}
