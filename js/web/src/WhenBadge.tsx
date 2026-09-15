import { createMemo, Show } from "solid-js";
import { formatWhenBadge, nowMs, todayStamp } from "./format.tsx";
import calendarSvg from "./icons/calendar.svg?raw";
import { useAppI18n } from "./i18n.tsx";

// Compact planned-date badge, the `when` twin of `DeadlineBadge`. Reads
// the raw register (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`) and renders a
// short label with a `data-tone` of today / future, or muted for
// done/binned items. A past `when` shows its date, never a word and never
// a tone: nothing is owed, and the useful fact is which day. Timed values
// append the wall-clock time in the preferred 12 / 24-hour cycle.
// Recomputes off the shared `nowMs()` tick so the tone rolls over at
// local midnight.
export function WhenBadge(props: { when: string; muted?: boolean }) {
  const { m, locale } = useAppI18n();
  const info = createMemo(() =>
    formatWhenBadge(
      props.when,
      todayStamp(nowMs()),
      { today: m().when.today, tomorrow: m().when.tomorrow },
      locale(),
    ),
  );
  const tone = () => (props.muted ? "muted" : (info()?.urgency ?? "future"));
  const title = () => `${m().when.label}: ${props.when}`;
  return (
    <Show when={info()}>
      {(i) => (
        <span class="badge when-badge" data-tone={tone()} title={title()}>
          <span class="deadline-badge-icon" innerHTML={calendarSvg} />
          {i().label}
        </span>
      )}
    </Show>
  );
}
