// A person as a small grayscale disc with the first letter of their
// email: item attribution in the item dialog on shared workspaces
// (`spec/sharing-plan.md` §Attribution), and later the "who's here" row
// by the workspace name.
// Deliberately colourless for now; a per-account hue can come later
// without changing callers. An unknown person (a member removed before
// this session loaded the list) renders "?" so the slot still reads as
// "someone else".

export function avatarInitial(email: string | null | undefined): string {
  const trimmed = (email ?? "").trim();
  if (!trimmed) return "?";
  // First code point, not first UTF-16 unit, so an emoji or non-BMP
  // letter doesn't render as half a character.
  const first = [...trimmed][0] ?? "?";
  return first.toLocaleUpperCase();
}

export function Avatar(props: { email: string | null; title?: string }) {
  return (
    <span
      class="avatar"
      role="img"
      aria-label={props.title}
      title={props.title}
      data-unknown={props.email ? undefined : ""}
    >
      {avatarInitial(props.email)}
    </span>
  );
}
