import { nowMs } from "./format.tsx";
import calendarSvg from "./icons/calendar.svg?raw";
import { useAppI18n } from "./i18n.tsx";
import { suggestionLabel } from "./whenSuggest.ts";

// A date suggestion (`whenSuggest.ts`): the date a phrase in the title
// resolved to, put as a question in the date's place in the task
// surface, in orange with a dashed edge so it never reads as a set
// date. Two answers: yes makes it the item's `when`, no drops it.
// A press on an answer leaves focus where it is (the title being typed).
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
    <span
      class="when-suggestion"
      role="group"
      aria-label={m().when.suggestionGroup}
      title={`${m().when.suggestionGroup}: ${props.when}`}
    >
      <span class="deadline-badge-icon" innerHTML={calendarSvg} />
      <span class="when-suggestion-label">{label()}</span>
      <span class="when-suggestion-answers" on:mousedown={(e) => e.preventDefault()}>
        <button
          type="button"
          class="when-suggestion-answer"
          data-answer="yes"
          tabIndex={0}
          on:click={() => props.onYes()}
        >
          {m().when.suggestionYes}
        </button>
        <button
          type="button"
          class="when-suggestion-answer"
          data-answer="no"
          tabIndex={0}
          on:click={() => props.onNo()}
        >
          {m().when.suggestionNo}
        </button>
      </span>
    </span>
  );
}
