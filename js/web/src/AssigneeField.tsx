// The task dialog's assignee control (`spec/sharing-plan.md`
// §Assignment): a row of its own showing who the item is assigned to,
// opening a menu of the workspace's members plus "Unassigned". One
// assignee: picking a member replaces the last. Promote-on-set, as the
// deadline band: the surface mounts it only while the item has an
// assignee or the "…" menu's "Assign" is being answered, which is why
// the menu's open state is the caller's. Shared workspaces only; the
// workspace decides (`assignmentOf`).

import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { For, Show } from "solid-js";
import { Avatar } from "./Avatar.tsx";
import checkSvg from "./icons/check.svg?raw";
import { useAppI18n } from "./i18n.tsx";

/** A person as the member directory knows them. `email` is null for an
 *  account no longer in the member list. */
export type Person = { accountId: string; email: string | null };

/** What the control needs for one item, resolved by the workspace. */
export type Assignment = {
  /** The doc's members, the session's own account first. */
  people: readonly Person[];
  /** The current assignee, or null when unassigned. May be someone not
   *  in `people` (a former member). */
  assignee: Person | null;
  /** The session's own account id, tagged "You" in the menu. */
  me: string | null;
};

/** Menu value standing for "no assignee". Not the empty string, which a
 *  radio group reads as "nothing selected"; an account id is a uuid, so
 *  it cannot collide. */
const NONE = "-";

export function AssigneeField(props: {
  assignment: () => Assignment;
  onChange: (accountId: string | null) => void;
  /** The member menu's open state, owned by the surface: its "…" menu
   *  opens it, and the row folds away when it closes with no assignee. */
  open: () => boolean;
  setOpen: (open: boolean) => void;
}) {
  const { m } = useAppI18n();
  const assignee = () => props.assignment().assignee;
  const nameOf = (p: Person): string => p.email ?? m().sharing.formerMember;
  // The members, plus the assignee when they are no longer one, so the
  // menu always shows the current value as a row.
  const options = (): readonly Person[] => {
    const { people, assignee: a } = props.assignment();
    return a && !people.some((p) => p.accountId === a.accountId) ? [...people, a] : people;
  };
  return (
    <DropdownMenu open={props.open()} onOpenChange={props.setOpen}>
      <DropdownMenu.Trigger
        class="task-dialog-assignee"
        tabIndex={0}
        data-empty={assignee() ? undefined : ""}
        aria-label={m().workspace.assignee}
        title={
          assignee() ? m().workspace.assignedTo(nameOf(assignee()!)) : m().workspace.assignee
        }
      >
        <Show when={assignee()} fallback={<span>{m().workspace.assign}</span>}>
          {(a) => (
            <>
              <Avatar accountId={a().accountId} email={a().email} />
              <span class="task-dialog-assignee-name">{nameOf(a())}</span>
            </>
          )}
        </Show>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content class="dropdown-menu-content task-dialog-assignee-menu">
          <DropdownMenu.RadioGroup
            value={assignee()?.accountId ?? NONE}
            onChange={(v) => props.onChange(v === NONE ? null : v)}
          >
            <DropdownMenu.RadioItem
              value={NONE}
              class="dropdown-menu-item task-dialog-assignee-item"
              closeOnSelect
            >
              <span class="task-dialog-assignee-name">{m().workspace.unassigned}</span>
              <DropdownMenu.ItemIndicator
                class="task-dialog-lifecycle-check"
                aria-hidden="true"
                innerHTML={checkSvg}
              />
            </DropdownMenu.RadioItem>
            <For each={options()}>
              {(p) => (
                <DropdownMenu.RadioItem
                  value={p.accountId}
                  class="dropdown-menu-item task-dialog-assignee-item"
                  closeOnSelect
                >
                  <Avatar accountId={p.accountId} email={p.email} />
                  <span class="task-dialog-assignee-name">{nameOf(p)}</span>
                  <Show when={p.accountId === props.assignment().me}>
                    <span class="task-dialog-assignee-you">{m().sharing.you}</span>
                  </Show>
                  <DropdownMenu.ItemIndicator
                    class="task-dialog-lifecycle-check"
                    aria-hidden="true"
                    innerHTML={checkSvg}
                  />
                </DropdownMenu.RadioItem>
              )}
            </For>
          </DropdownMenu.RadioGroup>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu>
  );
}
