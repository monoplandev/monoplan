// `assignee` end to end through the web store: the setter writes and
// clears the register, the change crosses the wasm boundary as a
// surgical `itemAssigneeChanged`, and an assignment already in the doc
// arrives with the boot snapshot (`spec/sharing-plan.md` §Assignment).

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";
import { Dek, Doc, Replica } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";
import { createMemberDirectory } from "../src/sync/members.ts";

const DOC_ID = "00000000-0000-0000-0000-00000000000b";

function appOver(doc: Doc) {
  return createSyncedApp(
    new Replica(
      doc,
      DOC_ID,
      Dek.generate(),
      0n,
      new MemReplicaStorage() as unknown as ReplicaStorage,
    ),
  );
}

describe("assignee", () => {
  test("assign, reassign and unassign through the store", () => {
    createRoot((dispose) => {
      const app = appOver(Doc.create());
      const id = app.addItem("inbox", "call the plumber");
      expect(app.getItem(id)?.assignee).toBeUndefined();
      app.setItemAssignee(id, "acct-1");
      expect(app.getItem(id)?.assignee).toBe("acct-1");
      app.setItemAssignee(id, "acct-2");
      expect(app.getItem(id)?.assignee).toBe("acct-2");
      app.setItemAssignee(id, null);
      expect(app.getItem(id)?.assignee).toBeUndefined();
      dispose();
    });
  });

  test("an assignment survives the boot snapshot path", () => {
    createRoot((dispose) => {
      const doc = Doc.create();
      const id = doc.addItem("inbox", "from before boot");
      doc.setItemAssignee(id, "acct-9");
      const app = appOver(doc);
      expect(app.getItem(id)?.assignee).toBe("acct-9");
      dispose();
    });
  });

  test("the member directory lists who can be assigned, by email", async () => {
    await createRoot(async (dispose) => {
      const dir = createMemberDirectory(async () => [
        { account_id: "b2", email: "bob@example.com", role: "member", added_at: 0 },
        { account_id: "a1", email: "ann@example.com", role: "owner", added_at: 0 },
      ]);
      expect(dir.listOf(DOC_ID)).toEqual([]);
      dir.ensure(DOC_ID);
      await new Promise((r) => setTimeout(r, 0));
      expect(dir.listOf(DOC_ID)).toEqual([
        { accountId: "a1", email: "ann@example.com" },
        { accountId: "b2", email: "bob@example.com" },
      ]);
      dispose();
    });
  });
});
