import { Show } from "solid-js";
import { nowMs } from "./format.tsx";
import calendarSvg from "./icons/calendar.svg?raw";
import checkSvg from "./icons/check.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { suggestionLabel } from "./whenSuggest.ts";

// A date suggestion (`whenSuggest.ts`): the date a phrase in the title
// resolved to, put as a question in the date's place, in orange with a
// dashed edge so it never reads as a set date. Two answers: yes makes
// it the item's `when`, no drops it.
//
// Two shapes. The block (task surface) spells the answers out as words;
// `compact` (list rows, board cards) is a badge-sized pill with a tick
// and a cross. With no handlers it is a bare preview: the draft row
// shows what the text will ask once the item exists.
//
// The buttons keep their events to themselves: on a row a click must
// not select, open or drag it, and in an editor a press must not take
// focus from the title being typed.
export function WhenSuggestion(props: {
  when: string;
  compact?: boolean;
  onYes?: () => void;
  onNo?: () => void;
}) {
  const { m, locale } = useAppI18n();
  const label = () =>
    m().when.suggestion(
      suggestionLabel(props.when, nowMs(), locale(), {
        today: m().when.today,
        tomorrow: m().when.tomorrow,
      }),
    );
  const answer = (fn: (() => void) | undefined) => (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    fn?.();
  };
  const swallow = (e: Event) => e.stopPropagation();
  const hold = (e: Event) => {
    // Keep focus where it is (the title editor), and the press off the row.
    e.preventDefault();
    e.stopPropagation();
  };
  return (
    <span
      class="when-suggestion"
      classList={{ badge: props.compact }}
      data-compact={props.compact ? "" : undefined}
      role="group"
      aria-label={m().when.suggestionGroup}
      title={`${m().when.suggestionGroup}: ${props.when}`}
    >
      <span class="deadline-badge-icon" innerHTML={calendarSvg} />
      <span class="when-suggestion-label">{label()}</span>
      <Show when={props.onYes || props.onNo}>
        <span
          class="when-suggestion-answers"
          on:pointerdown={swallow}
          on:mousedown={hold}
          on:dblclick={swallow}
        >
          <button
            type="button"
            class="when-suggestion-answer"
            data-answer="yes"
            tabIndex={0}
            aria-label={m().when.suggestionYes}
            title={m().when.suggestionYes}
            on:click={answer(props.onYes)}
          >
            <Show when={props.compact} fallback={m().when.suggestionYes}>
              <span class="when-suggestion-glyph" innerHTML={checkSvg} />
            </Show>
          </button>
          <button
            type="button"
            class="when-suggestion-answer"
            data-answer="no"
            tabIndex={0}
            aria-label={m().when.suggestionNo}
            title={m().when.suggestionNo}
            on:click={answer(props.onNo)}
          >
            <Show when={props.compact} fallback={m().when.suggestionNo}>
              <span class="when-suggestion-glyph" aria-hidden="true">
                ✕
              </span>
            </Show>
          </button>
        </span>
      </Show>
    </span>
  );
}
