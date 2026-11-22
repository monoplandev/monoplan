import { Show } from "solid-js";
import mapPinSvg from "./icons/map-pin.svg?raw";
import type { Place } from "./sync/store.ts";

// Compact place pill: the pin glyph and the label, in the planned-date
// pill's geometry. The label is its own span so it can ellipsize when a
// tight row squeezes the pill (`.place-badge` in styles.css). The title
// carries the formatted address when the place came from a lookup, so
// hovering reveals where "Luigi's" actually is. Muted for done/binned
// items like the date pills. `iconOnly` (list rows) shows just the pin;
// the label moves into the title.
export function PlaceBadge(props: { place: Place; muted?: boolean; iconOnly?: boolean }) {
  const title = () => {
    const { label, address } = props.place;
    if (!props.iconOnly) return address ?? label;
    return address && address !== label ? `${label} — ${address}` : label;
  };
  return (
    <span
      class="badge place-badge"
      classList={{ "badge-icon-only": props.iconOnly }}
      data-tone={props.muted ? "muted" : undefined}
      title={title()}
      aria-label={props.iconOnly ? title() : undefined}
    >
      <span class="deadline-badge-icon" innerHTML={mapPinSvg} />
      <Show when={!props.iconOnly}>
        <span class="place-badge-label">{props.place.label}</span>
      </Show>
    </span>
  );
}
