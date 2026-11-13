import { Popover } from "@kobalte/core/popover";
import { createSignal, Show } from "solid-js";

import { EmojiPicker } from "./EmojiPicker.tsx";
import fileSvg from "./icons/file.svg?raw";
import { useAppI18n } from "./i18n.tsx";

/** Header affordance for a user-created list: renders the list's icon
 *  (the chosen emoji, or the default file glyph when unset) and opens a
 *  popover holding the searchable emoji picker. Icons are stored verbatim
 *  as the literal grapheme (see `Doc::set_list_icon`).
 *  Reserved `main` (Home) has no `ListMeta` row and never renders this. */
export function ListIconPicker(props: {
  icon: string | undefined;
  onPick: (icon: string) => void;
  onClear: () => void;
}): ReturnType<typeof Popover> {
  const { m } = useAppI18n();
  const [open, setOpen] = createSignal(false);

  return (
    <Popover open={open()} onOpenChange={setOpen} placement="bottom-start" gutter={6}>
      <Popover.Trigger
        class="icon-button"
        tabIndex={-1}
        aria-label={m().workspace.listIcon}
      >
        <Show when={props.icon} fallback={<span innerHTML={fileSvg} />}>
          {(icon) => <span class="list-icon-emoji">{icon()}</span>}
        </Show>
      </Popover.Trigger>
      <Popover.Portal>
        {/* The picker manages its own focus (search field on open), so hand
            it the initial focus rather than letting Popover target the
            content element. */}
        <Popover.Content class="list-icon-popover" onOpenAutoFocus={(e) => e.preventDefault()}>
          <EmojiPicker
            selected={props.icon}
            onPick={(icon) => {
              props.onPick(icon);
              setOpen(false);
            }}
            onClear={() => {
              props.onClear();
              setOpen(false);
            }}
          />
        </Popover.Content>
      </Popover.Portal>
    </Popover>
  );
}
