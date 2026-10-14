// Draft items live only in the dnd's items list — never in the engine —
// until the user commits them. The id prefix is the discriminator the
// Row uses to switch between "edit existing" and "create new" save paths
// on collapse; the context menu uses it to keep drafts out of Focus.
export const DRAFT_ID_PREFIX = "__draft__";
export const isDraftId = (id: string): boolean => id.startsWith(DRAFT_ID_PREFIX);
