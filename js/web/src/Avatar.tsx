// A person as a small coloured disc with the first letter of their email:
// item attribution in the item dialog on shared workspaces
// (`spec/sharing-plan.md` §Attribution) and the "who's here" row by the
// workspace name. The hue is a pure function of the account id, so the
// same person is the same colour on every device and in every session,
// with no stored preference. An unknown person (a member removed before
// this session loaded the list) renders a grey dashed "?" so the slot
// still reads as "someone else".

const COLOURS: Record<string, string> = {
  salmon: "#FF928B",
  tangerine: "#FFD9C6",
  peach: "#FEC3A6",
  vanilla: "#FEF2AD",
  lavender: "#E2BDFE",
  lemon: "#FCF5C7",
  gold: "#FFEE93",
  blue: "#9DC4FF",
  gween: "#A2FFC2",
  celadon: "#ADF7B6",
};

const KEYS = Object.keys(COLOURS);

/** Disc colour for an account. Account ids are hyphenated uuids, so the
 *  first 8 chars are hex; a non-hex id (tests, future id schemes) falls
 *  back to a byte sum so the pick stays deterministic rather than NaN. */
export function avatarColour(accountId: string): string {
  let n = parseInt(accountId.slice(0, 8), 16);
  if (!Number.isFinite(n)) {
    n = 0;
    for (const ch of accountId) n = (n + ch.codePointAt(0)!) >>> 0;
  }
  return COLOURS[KEYS[n % KEYS.length]];
}

export function avatarInitial(email: string | null | undefined): string {
  const trimmed = (email ?? "").trim();
  if (!trimmed) return "?";
  // First code point, not first UTF-16 unit, so an emoji or non-BMP
  // letter doesn't render as half a character.
  const first = [...trimmed][0] ?? "?";
  return first.toLocaleUpperCase();
}

export function Avatar(props: { accountId: string; email: string | null; title?: string }) {
  return (
    <span
      class="avatar"
      role="img"
      aria-label={props.title}
      title={props.title}
      data-unknown={props.email ? undefined : ""}
      data-tinted={props.email ? "" : undefined}
      style={props.email ? { "--avatar-bg": avatarColour(props.accountId) } : undefined}
    >
      {avatarInitial(props.email)}
    </span>
  );
}
