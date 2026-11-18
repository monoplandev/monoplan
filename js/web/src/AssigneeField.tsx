// The task dialog's assignee control (`spec/sharing-plan.md`
// §Assignment): a row of its own showing who the item is assigned to,
// opening a menu of the workspace's members. One assignee: picking a
// member replaces the last, and the inset ✕ (hover reveal, as the date
// and place fields') unassigns. Always there, as the
// date and place bands and the Mac item panel's row: unassigned it
// reads a muted "Assign" behind a person glyph. Shared workspaces only;
// the workspace decides (`assignmentOf`).

import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { For, Show } from "solid-js";
import { Avatar } from "./Avatar.tsx";
import checkSvg from "./icons/check.svg?raw";
import personSvg from "./icons/person.svg?raw";
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

export function AssigneeField(props: {
  assignment: () => Assignment;
  onChange: (accountId: string | null) => void;
  /** The member menu's open state, owned by the surface, which closes
   *  it when the pane moves to another item. */
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
    <span class="task-dialog-when-anchor">
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
          <Show
            when={assignee()}
            fallback={
              <>
                <span class="task-dialog-assignee-icon" aria-hidden="true" innerHTML={personSvg} />
                <span>{m().workspace.assign}</span>
              </>
            }
          >
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
              value={assignee()?.accountId ?? ""}
              onChange={(v) => props.onChange(v)}
            >
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
      <Show when={assignee()}>
        <button
          type="button"
          class="icon-button task-dialog-place-action"
          aria-label={m().workspace.unassign}
          title={m().workspace.unassign}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => props.onChange(null)}
        >
          ✕
        </button>
      </Show>
    </span>
  );
}
