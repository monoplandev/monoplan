import { Dialog } from "@kobalte/core/dialog";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { For, Show, createEffect, createSignal, on } from "solid-js";
import { InviteSecret } from "@monoplan/core/wasm";
import { api, type Invite, type Member } from "./api.ts";
import { Avatar } from "./Avatar.tsx";
import { ConfirmDialog } from "./ConfirmDialog.tsx";
import { formatRelative } from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import dotsHorizontalSvg from "./icons/dots-horizontal.svg?raw";
import { closeToItems, trackOverlay } from "./overlay.ts";
import { useSession } from "./SessionContext.tsx";
import { inviteUrl } from "./url.ts";

/** Sharing for a non-Home workspace, opened from its heading menu
 *  (`spec/sharing-plan.md` §Invites). Lists the members, and for the
 *  owner issues single-use links: a fresh secret is generated here, the
 *  doc's DEK wrapped under the invite KEK, and only the HKDF id and the
 *  wrap go to the server, so the link shown below is the only copy of
 *  the secret that will ever exist. Unused links can be revoked. An
 *  owner can also remove a member from their row's "..." menu (never
 *  themself, never the last owner; confirmed, since revocation is
 *  forward-only — §Member removal). Same portal / overlay / positioner
 *  shape as `ConfirmDialog`. */
export function SharingDialog(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  /** The workspace's display name. */
  label: string;
  docId: string;
}) {
  const { m, locale } = useAppI18n();
  const session = useSession();
  trackOverlay(() => props.open);

  const [members, setMembers] = createSignal<Member[] | null>(null);
  const [invites, setInvites] = createSignal<Invite[] | null>(null);
  const [loadError, setLoadError] = createSignal(false);
  const [link, setLink] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [createError, setCreateError] = createSignal(false);
  const [copied, setCopied] = createSignal(false);
  const [revoking, setRevoking] = createSignal<Set<string>>(new Set());
  const [removing, setRemoving] = createSignal<Member | null>(null);

  const me = (): string => session.session().accountId;
  const isOwner = (): boolean =>
    members()?.some((mb) => mb.account_id === me() && mb.role === "owner") ?? false;
  const ownerCount = (): number =>
    members()?.filter((mb) => mb.role === "owner").length ?? 0;
  /** Removable by the viewer: they own the doc, the row is not theirs
   *  (leaving is a different verb), and the row is not the only owner. */
  const canRemove = (mb: Member): boolean =>
    isOwner() && mb.account_id !== me() && !(mb.role === "owner" && ownerCount() <= 1);
  const isLastOwner = (mb: Member): boolean => mb.role === "owner" && ownerCount() <= 1;

  const load = async (): Promise<void> => {
    setLoadError(false);
    try {
      const { members: list } = await api.listMembers(props.docId);
      setMembers(list);
      if (list.some((mb) => mb.account_id === me() && mb.role === "owner")) {
        const { invites: issued } = await api.listInvites(props.docId);
        setInvites(issued);
      } else {
        setInvites(null);
      }
    } catch (e) {
      console.warn("sharing: load failed:", e);
      setLoadError(true);
    }
  };

  // Fresh state every time the dialog opens; the link shown is this
  // open's only copy, so it does not carry over.
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (!open) return;
        setMembers(null);
        setInvites(null);
        setLink(null);
        setCreateError(false);
        setCopied(false);
        setRemoving(null);
        void load();
      },
    ),
  );

  const removeMember = async (mb: Member): Promise<void> => {
    try {
      await api.removeMember(props.docId, mb.account_id);
      setMembers((list) => list?.filter((x) => x.account_id !== mb.account_id) ?? null);
    } catch (e) {
      console.error("sharing: remove member failed:", e);
      alert(m().sharing.removeMemberFailed);
      // Whatever the server now thinks is the truth.
      void load();
    }
  };

  const createLink = async (): Promise<void> => {
    const dek = session.session().docKeys.get(props.docId);
    if (!dek || creating()) return;
    setCreating(true);
    setCreateError(false);
    setCopied(false);
    try {
      const secret = InviteSecret.generate();
      const wrapped = secret.wrap(dek);
      await api.createInvite(props.docId, {
        invite_id: secret.id(),
        wrapped_dek: wrapped.ciphertext,
        wrapped_dek_nonce: wrapped.nonce,
      });
      setLink(inviteUrl(secret.toHex()));
      const { invites: issued } = await api.listInvites(props.docId);
      setInvites(issued);
    } catch (e) {
      console.error("sharing: create link failed:", e);
      setCreateError(true);
    } finally {
      setCreating(false);
    }
  };

  const copyLink = async (): Promise<void> => {
    const url = link();
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
    } catch (e) {
      console.warn("sharing: copy failed:", e);
    }
  };

  const revoke = async (inviteId: string): Promise<void> => {
    setRevoking((s) => new Set(s).add(inviteId));
    try {
      await api.revokeInvite(props.docId, inviteId);
      setInvites((list) => list?.filter((i) => i.invite_id !== inviteId) ?? null);
    } catch (e) {
      console.error("sharing: revoke failed:", e);
      alert(m().sharing.revokeFailed);
    } finally {
      setRevoking((s) => {
        const next = new Set(s);
        next.delete(inviteId);
        return next;
      });
    }
  };

  const now = (): number => Date.now();
  const openInvites = (): Invite[] =>
    (invites() ?? []).filter((i) => i.consumed_at === null && i.expires_at > now());

  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content
            class="confirm-dialog sharing-dialog"
            onCloseAutoFocus={closeToItems}
          >
            <section class="confirm-dialog-section confirm-dialog-header">
              <Dialog.Title class="confirm-dialog-title">
                {m().sharing.title(props.label)}
              </Dialog.Title>
              <Dialog.Description class="confirm-dialog-message">
                {m().sharing.description}
              </Dialog.Description>
            </section>

            <section class="confirm-dialog-section sharing-section">
              <h3 class="sharing-heading">
                {m().sharing.members}
                <Show when={members()}>
                  {(list) => <span class="sharing-heading-count">({list().length})</span>}
                </Show>
              </h3>
              <Show
                when={members()}
                fallback={
                  <p class="confirm-dialog-message">
                    {loadError() ? m().sharing.loadFailed : m().common.loading}
                  </p>
                }
              >
                {(list) => (
                  <ul class="sharing-list">
                    <For each={list()}>
                      {(mb) => (
                        <li class="sharing-row">
                          {/* Same disc as the presence row and item
                              attribution, so the colour is learnable
                              from the member list. */}
                          <Avatar accountId={mb.account_id} email={mb.email} />
                          <span class="sharing-row-main">{mb.email}</span>
                          <span class="sharing-tags">
                            <Show when={mb.account_id === me()}>
                              <span class="device-current-tag">{m().sharing.you}</span>
                            </Show>
                            <Show when={mb.role === "owner"}>
                              <span class="device-current-tag">{m().sharing.owner}</span>
                            </Show>
                          </span>
                          {/* Owners get a per-row menu on every row but
                              their own. The item is disabled, not hidden,
                              on a last-owner row so the rule is visible. */}
                          <Show when={isOwner() && mb.account_id !== me()}>
                            <DropdownMenu placement="bottom-end" gutter={4}>
                              <DropdownMenu.Trigger
                                class="icon-button sharing-row-menu"
                                aria-label={m().sharing.memberMenu}
                                title={m().sharing.memberMenu}
                                innerHTML={dotsHorizontalSvg}
                              />
                              <DropdownMenu.Portal>
                                <DropdownMenu.Content class="dropdown-menu-content sharing-member-menu">
                                  <DropdownMenu.Item
                                    class="dropdown-menu-item"
                                    disabled={!canRemove(mb)}
                                    title={isLastOwner(mb) ? m().sharing.lastOwner : undefined}
                                    onSelect={() => {
                                      // Let the menu close and restore focus
                                      // before the confirm dialog takes it.
                                      requestAnimationFrame(() => setRemoving(mb));
                                    }}
                                  >
                                    {m().sharing.removeMember}
                                  </DropdownMenu.Item>
                                </DropdownMenu.Content>
                              </DropdownMenu.Portal>
                            </DropdownMenu>
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                )}
              </Show>
            </section>

            <Show when={members() && !loadError()}>
              <section class="confirm-dialog-section sharing-section">
                <h3 class="sharing-heading">{m().sharing.inviteLink}</h3>
                <Show
                  when={isOwner()}
                  fallback={<p class="confirm-dialog-message">{m().sharing.ownerOnly}</p>}
                >
                  <p class="confirm-dialog-message">{m().sharing.linkHint}</p>
                  <Show
                    when={link()}
                    fallback={
                      <button
                        type="button"
                        class="confirm-dialog-btn confirm-dialog-confirm sharing-create"
                        disabled={creating()}
                        onClick={() => void createLink()}
                      >
                        {creating() ? m().sharing.creating : m().sharing.createLink}
                      </button>
                    }
                  >
                    {(url) => (
                      <div class="sharing-link-row">
                        <input
                          class="name-dialog-input sharing-link-input"
                          type="text"
                          readOnly
                          value={url()}
                          aria-label={m().sharing.inviteLink}
                          onFocus={(e) => e.currentTarget.select()}
                        />
                        <button
                          type="button"
                          class="confirm-dialog-btn confirm-dialog-confirm"
                          onClick={() => void copyLink()}
                        >
                          {copied() ? m().sharing.copied : m().common.copy}
                        </button>
                      </div>
                    )}
                  </Show>
                  <Show when={createError()}>
                    <p class="name-dialog-error" role="alert">
                      {m().sharing.createFailed}
                    </p>
                  </Show>
                  <Show when={openInvites().length > 0}>
                    <h4 class="sharing-subheading">{m().sharing.pendingLinks}</h4>
                    <ul class="sharing-list">
                      <For each={openInvites()}>
                        {(inv) => (
                          <li class="sharing-row">
                            <span class="sharing-row-main sharing-row-muted">
                              {m().sharing.createdAgo(
                                formatRelative(inv.created_at, now(), locale()),
                              )}
                            </span>
                            <button
                              type="button"
                              class="confirm-dialog-btn confirm-dialog-cancel sharing-revoke"
                              disabled={revoking().has(inv.invite_id)}
                              onClick={() => void revoke(inv.invite_id)}
                            >
                              {m().sharing.revoke}
                            </button>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                </Show>
              </section>
            </Show>

            <section class="confirm-dialog-section confirm-dialog-actions">
              <button
                type="button"
                class="confirm-dialog-btn confirm-dialog-cancel"
                onClick={() => props.onOpenChange(false)}
              >
                {m().common.close}
              </button>
            </section>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
      <ConfirmDialog
        open={removing() !== null}
        onOpenChange={(b) => {
          if (!b) setRemoving(null);
        }}
        title={m().sharing.removeMemberTitle}
        message={m().sharing.removeMemberConfirm(removing()?.email ?? "")}
        confirmLabel={m().sharing.removeMember}
        onConfirm={() => {
          const mb = removing();
          setRemoving(null);
          if (mb) void removeMember(mb);
        }}
      />
    </Dialog>
  );
}
