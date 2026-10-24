import { ask, store } from "../../../store";
import type { StagedChange } from "../../../ipc/types";

/** Discards a staged change once the person confirms. `edits` counts what is lost; `title`
 *  and `ok` name the question and its button after the control that asked. */
export async function discardWithAsk(c: StagedChange, { edits = c.edits.length, title = `Discard ${c.title}?`, ok = "Discard" } = {}) {
  const lost = edits === 1 ? "Its 1 edit is lost." : `Its ${edits} edits are lost.`;
  if ((await ask(title, lost, { ok, danger: true })) !== true) return;
  await store.getState().discardChange(c.id);
}
