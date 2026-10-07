import mapPinSvg from "./icons/map-pin.svg?raw";
import type { Place } from "./sync/store.ts";

// Compact place pill: the pin glyph and the label, in the planned-date
// pill's geometry. The title carries the formatted address when the place
// came from a lookup, so hovering reveals where "Luigi's" actually is.
// Muted for done/binned items like the date pills.
export function PlaceBadge(props: { place: Place; muted?: boolean }) {
  return (
    <span
      class="badge place-badge"
      data-tone={props.muted ? "muted" : undefined}
      title={props.place.address ?? props.place.label}
    >
      <span class="deadline-badge-icon" innerHTML={mapPinSvg} />
      {props.place.label}
    </span>
  );
}
