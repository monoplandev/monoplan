// What a row says about attachments (`spec/attachments.md`): a size in
// the task surface, a count on a list row. Apart from `FilesField` so a
// list row and the tests need none of its menu machinery.

import type { ItemFile } from "./sync/store.ts";

/** A file size for a row: bytes, KB or MB (1024s), at most one decimal. */
export function formatFileSize(bytes: number, locale: string): string {
  const fmt = (n: number, digits: number): string =>
    new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(n);
  if (bytes < 1024) return `${fmt(bytes, 0)} B`;
  if (bytes < 1024 * 1024) return `${fmt(bytes / 1024, 0)} KB`;
  return `${fmt(bytes / (1024 * 1024), 1)} MB`;
}

/** The count a list row carries for an item's attachments, or null when
 *  it has none. */
export function fileCount(files: readonly ItemFile[] | undefined): number | null {
  return files && files.length > 0 ? files.length : null;
}
