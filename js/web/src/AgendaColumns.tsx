// PROTOTYPE: the agenda laid out horizontally, one column per day
// (Overdue first when there is one), scrolling sideways. Fed the same
// `DayGroup`s as `Upcoming`; not windowed, no drag, no draft row, no
// context menu. Click selects (the side pane follows), double-click
// opens, the checkbox ticks. Toggled from the agenda's filter menu.

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { pressCheck } from "./Row.tsx";
import { agendaItems, type DayGroup, type DayRow } from "./dayGroups.ts";
import type { DndSelection } from "./dnd";
import { formatClockTime, whenTime } from "./format.tsx";
import { useAppI18n } from "./i18n.tsx";
import { isCancelled, isClosed, isEvent, type ItemActions } from "./sync/store.ts";

export function AgendaColumns(props: {
  app: ItemActions;
  groups: DayGroup[];
  selection: DndSelection;
  onOpen: (id: string) => void;
}) {
  const { m, locale } = useAppI18n();
  let hostEl!: HTMLDivElement;

  const order = createMemo(() => agendaItems(props.groups).map((r) => r.item.id));
  createEffect(() => props.selection.updateOrder(order()));

  const [selTick, setSelTick] = createSignal(0, { equals: false });
  onCleanup(props.selection.onChange(() => setSelTick((n) => n + 1)));
  const isSelected = (id: string) => {
    selTick();
    return props.selection.getSelectedKeySet().has(id);
  };

  const timeLabel = (r: DayRow): string => {
    if (r.placedBy === "deadline") return m().upcoming.due;
    if (r.placedBy !== "when") return "";
    const start = whenTime(r.item.when!);
    if (start) return formatClockTime(start, locale());
    return isEvent(r.item) ? m().when.allDay : "";
  };

  // Open with Overdue (else Today) at the left edge; the past is a
  // scroll left away.
  onMount(() => {
    requestAnimationFrame(() => {
      const el =
        hostEl.querySelector<HTMLElement>('[data-urgency="overdue"]') ??
        hostEl.querySelector<HTMLElement>('[data-urgency="today"]');
      if (el) hostEl.scrollLeft = el.offsetLeft - 12;
    });
  });

  // A mouse wheel (vertical only) over a column header scrolls the days.
  const onWheel = (e: WheelEvent) => {
    if (e.deltaX !== 0 || !(e.target as HTMLElement).closest(".agenda-col-header")) return;
    hostEl.scrollLeft += e.deltaY;
    e.preventDefault();
  };

  return (
    <div class="agenda-cols" ref={hostEl} onWheel={onWheel}>
      <For each={props.groups}>
        {(g) => (
          <section class="agenda-col" data-urgency={g.urgency}>
            <header class="agenda-col-header">
              <span class="agenda-col-label">{g.label}</span>
              <Show when={g.rows.length > 0}>
                <span class="agenda-col-count">{g.rows.length}</span>
              </Show>
            </header>
            <div class="agenda-col-body">
              <For each={g.rows} fallback={<div class="agenda-col-empty">—</div>}>
                {(r) => (
                  <div
                    class="agenda-card"
                    aria-selected={isSelected(r.item.id) ? "true" : "false"}
                    data-closed={isClosed(r.item) ? "" : undefined}
                    data-event={isEvent(r.item) ? "" : undefined}
                    onClick={() => props.selection.selectOnly(r.item.id)}
                    onDblClick={() => props.onOpen(r.item.id)}
                  >
                    <Show
                      when={!isEvent(r.item) || r.placedBy !== "when" || isClosed(r.item)}
                      fallback={<span class="agenda-card-pill" aria-hidden="true" />}
                    >
                      <input
                        type="checkbox"
                        class="task-check"
                        checked={isClosed(r.item)}
                        data-cancelled={isCancelled(r.item) ? "" : undefined}
                        aria-label={m().workspace.markDone}
                        onClick={(e) => e.stopPropagation()}
                        onPointerDown={(e) => pressCheck(e.currentTarget)}
                        onChange={(e) => props.app.setDone(r.item.id, e.currentTarget.checked)}
                      />
                    </Show>
                    <div class="agenda-card-body">
                      <Show when={timeLabel(r)}>
                        <div class="agenda-card-time">{timeLabel(r)}</div>
                      </Show>
                      <div class="agenda-card-text">{r.item.text}</div>
                      <Show when={r.item.place?.label}>
                        <div class="agenda-card-sub">{r.item.place!.label}</div>
                      </Show>
                    </div>
                  </div>
                )}
              </For>
            </div>
          </section>
        )}
      </For>
    </div>
  );
}
