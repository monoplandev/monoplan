import { nowMs } from "./format.tsx";
import calendarSvg from "./icons/calendar.svg?raw";
import checkSvg from "./icons/check.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { suggestionLabel } from "./whenSuggest.ts";

// A date suggestion (`whenSuggest.ts`): the date a phrase in the title
// resolved to, standing in the date's place in the task surface. It has
// the date input's anatomy (`WhenField`: the calendar glyph inset on
// the left, a 32px line of text in the surface's own font, controls
// inset on the right) and reads "Suggested: Fri 14 Nov 08:30" in
// orange, so it is never taken for a set date. Two answers at the right
// edge: a tick makes it the item's `when`, a ✕ drops it. A press on
// either leaves focus where it is (the title being typed).
export function WhenSuggestion(props: {
  when: string;
  onYes: () => void;
  onNo: () => void;
}) {
  const { m, locale } = useAppI18n();
  const label = () =>
    m().when.suggestion(
      suggestionLabel(props.when, nowMs(), locale(), {
        today: m().when.today,
        tomorrow: m().when.tomorrow,
      }),
    );
  return (
    <span class="task-dialog-when-anchor when-suggestion" role="group" aria-label={label()}>
      <span class="task-dialog-when-icon" aria-hidden="true" innerHTML={calendarSvg} />
      <span class="when-suggestion-label">{label()}</span>
      <span class="when-suggestion-answers" on:mousedown={(e) => e.preventDefault()}>
        <button
          type="button"
          class="icon-button when-suggestion-answer"
          data-answer="yes"
          tabIndex={0}
          aria-label={m().when.suggestionYes}
          title={m().when.suggestionYes}
          on:click={() => props.onYes()}
          innerHTML={checkSvg}
        />
        <button
          type="button"
          class="icon-button when-suggestion-answer"
          data-answer="no"
          tabIndex={0}
          aria-label={m().when.suggestionNo}
          title={m().when.suggestionNo}
          on:click={() => props.onNo()}
        >
          ✕
        </button>
      </span>
    </span>
  );
}
