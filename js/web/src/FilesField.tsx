// The task surface's attachments (`spec/attachments.md`): the item's
// files by name and size, over a trailing "Attach file" row. The bytes
// are on the server: a click on a name fetches the file and saves it,
// and nothing is fetched before that. Each row's menu downloads or
// deletes; a delete is undone like any other edit. A file keeps the
// name it was attached under.
//
// A file being uploaded shows as a row of its own under the list until
// its entry lands in the doc (upload first, reference second), and
// stays there with the reason if the upload fails.
//
// Opening always saves to disk: a fetched file is never rendered in
// this origin, whatever its type claims to be, because any member of a
// shared workspace chooses that type.

import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { createSignal, For, Show } from "solid-js";
import { useAppI18n, type Messages } from "./i18n.tsx";
import dotsHorizontalSvg from "./icons/dots-horizontal.svg?raw";
import fileSvg from "./icons/file.svg?raw";
import link2Svg from "./icons/link-2.svg?raw";
import spinnerSvg from "./icons/spinner.svg?raw";
import { formatFileSize } from "./fileFormat.ts";
import { FileError, MAX_FILE_BYTES, type FileErrorKind } from "./sync/files.ts";
import type { ItemFile } from "./sync/store.ts";

type Pending = { key: number; name: string; error?: FileErrorKind };

export function FilesField(props: {
  files: () => readonly ItemFile[];
  /** False on a signed-out session: the files it has (none, in
   *  practice) still list, nothing can be attached. */
  canAttach: boolean;
  /** Upload and attach; rejects with a `FileError`. */
  onAttach: (file: File) => Promise<unknown>;
  /** The file's bytes; rejects with a `FileError`. */
  onRead: (fileId: string) => Promise<Uint8Array>;
  onRemove: (fileId: string) => void;
}) {
  const { m, locale } = useAppI18n();
  let pickRef: HTMLInputElement | undefined;
  let nextKey = 0;
  const [pending, setPending] = createSignal<readonly Pending[]>([]);
  // Per-file transient state: a fetch in flight, or why the last one
  // failed.
  const [busy, setBusy] = createSignal<ReadonlySet<string>>(new Set());
  const [failed, setFailed] = createSignal<Readonly<Record<string, FileErrorKind>>>({});

  const errorKind = (e: unknown): FileErrorKind => (e instanceof FileError ? e.kind : "failed");

  const attach = (file: File): void => {
    const key = nextKey++;
    const row: Pending = { key, name: file.name };
    // Refused before a byte is read.
    if (file.size > MAX_FILE_BYTES) {
      setPending((p) => [...p, { ...row, error: "tooLarge" }]);
      return;
    }
    setPending((p) => [...p, row]);
    props.onAttach(file).then(
      () => setPending((p) => p.filter((r) => r.key !== key)),
      (e) => {
        if (errorKind(e) === "failed") console.error("attach failed:", e);
        setPending((p) => p.map((r) => (r.key === key ? { ...r, error: errorKind(e) } : r)));
      },
    );
  };

  const open = (file: ItemFile): void => {
    if (busy().has(file.id)) return;
    setBusy((b) => new Set(b).add(file.id));
    setFailed(({ [file.id]: _gone, ...rest }) => rest);
    props
      .onRead(file.id)
      .then(
        (bytes) => saveBytes(file.name, bytes),
        (e) => {
          if (errorKind(e) === "failed") console.error("file open failed:", e);
          setFailed((f) => ({ ...f, [file.id]: errorKind(e) }));
        },
      )
      .finally(() =>
        setBusy((b) => {
          const next = new Set(b);
          next.delete(file.id);
          return next;
        }),
      );
  };

  return (
    <div class="files" role="group" aria-label={m().workspace.attachments}>
      <For each={props.files()}>
        {(file) => (
          <div class="file-row" data-failed={failed()[file.id] ? "" : undefined}>
            <span class="file-lead" aria-hidden="true">
              <Show
                when={busy().has(file.id)}
                fallback={<span class="file-icon" innerHTML={fileSvg} />}
              >
                <span class="file-icon file-spinner" innerHTML={spinnerSvg} />
              </Show>
            </span>
            <button
              type="button"
              class="file-name"
              title={m().workspace.fileSave(file.name)}
              onClick={() => open(file)}
            >
              {file.name}
            </button>
            <span class="file-meta">
              <Show when={failed()[file.id]} fallback={formatFileSize(file.size, locale())}>
                {(kind) => fileErrorText(m(), kind())}
              </Show>
            </span>
            <DropdownMenu>
              <DropdownMenu.Trigger
                class="icon-button file-menu"
                aria-label={m().workspace.fileActions(file.name)}
                innerHTML={dotsHorizontalSvg}
              />
              <DropdownMenu.Portal>
                <DropdownMenu.Content class="dropdown-menu-content">
                  <DropdownMenu.Item class="dropdown-menu-item" onSelect={() => open(file)}>
                    {m().workspace.fileDownload}
                  </DropdownMenu.Item>
                  <DropdownMenu.Item
                    class="dropdown-menu-item"
                    onSelect={() => props.onRemove(file.id)}
                  >
                    {m().common.delete}
                  </DropdownMenu.Item>
                </DropdownMenu.Content>
              </DropdownMenu.Portal>
            </DropdownMenu>
          </div>
        )}
      </For>
      <For each={pending()}>
        {(row) => (
          <div class="file-row file-pending" data-failed={row.error ? "" : undefined}>
            <span class="file-lead" aria-hidden="true">
              <Show when={!row.error} fallback={<span class="file-icon" innerHTML={fileSvg} />}>
                <span class="file-icon file-spinner" innerHTML={spinnerSvg} />
              </Show>
            </span>
            <span class="file-name">{row.name}</span>
            <span class="file-meta" role={row.error ? "alert" : "status"}>
              {row.error ? fileErrorText(m(), row.error) : m().workspace.fileUploading}
            </span>
            <Show when={row.error}>
              <button
                type="button"
                class="icon-button file-dismiss"
                aria-label={m().common.close}
                onClick={() => setPending((p) => p.filter((r) => r.key !== row.key))}
              >
                ×
              </button>
            </Show>
          </div>
        )}
      </For>
      <Show when={props.canAttach}>
        <button type="button" class="file-row file-add" onClick={() => pickRef?.click()}>
          <span class="file-lead" aria-hidden="true">
            <span class="file-icon" innerHTML={link2Svg} />
          </span>
          {m().workspace.attachFile}
        </button>
        <input
          ref={pickRef}
          type="file"
          multiple
          hidden
          tabIndex={-1}
          onChange={(e) => {
            const el = e.currentTarget;
            for (const file of Array.from(el.files ?? [])) attach(file);
            // So picking the same file again fires a change.
            el.value = "";
          }}
        />
      </Show>
    </div>
  );
}

function fileErrorText(m: Messages, kind: FileErrorKind): string {
  const w = m.workspace;
  switch (kind) {
    case "tooLarge":
      return w.fileTooLarge;
    case "quota":
      return w.fileQuota;
    case "offline":
      return w.fileOffline;
    case "unavailable":
      return w.fileUnavailable;
    case "signedOut":
    case "failed":
      return w.fileFailed;
  }
}

/** Hand fetched bytes to the browser as a download named `name`. The
 *  blob is typed as opaque bytes so nothing renders it in this origin. */
function saveBytes(name: string, bytes: Uint8Array): void {
  const url = URL.createObjectURL(
    new Blob([bytes as BlobPart], { type: "application/octet-stream" }),
  );
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.rel = "noopener";
  a.style.display = "none";
  document.body.append(a);
  a.click();
  a.remove();
  // The download reads the blob after the click returns.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
