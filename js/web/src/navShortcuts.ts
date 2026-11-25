// Digit shortcuts for the fixed nav views, in sidebar order: 1 Upcoming,
// 2 Focus, 3 Done, 4 Inbox. Pure decision logic, extracted from
// Workspace so it can be unit-tested without a DOM. Bin deliberately has
// no digit: its row only exists while it holds items, and it's a
// destination you reach via Backspace on an item, not a place to jump to.
// Any modifier disqualifies the key (Cmd+1 belongs to the browser); the
// editable-surface and overlay guards live centrally in `onGlobalKey`.

import type { ViewKey } from "./prefs.ts";

export function digitNavTarget(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">,
): ViewKey | null {
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return null;
  switch (e.key) {
    case "1":
      return { kind: "upcoming" };
    case "2":
      return { kind: "focus" };
    case "3":
      return { kind: "done" };
    case "4":
      return { kind: "list", id: "inbox" };
    default:
      return null;
  }
}
