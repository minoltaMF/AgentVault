import type { ConversationPreviewRow } from "./conversationDisplay";

/** Fetch a bounded window around the ordinal offset, never confuse it with a source line index. */
export function previewWindowStart(offset: number, pageSize: number): number {
  return Math.max(0, offset - Math.floor(pageSize / 2));
}

/** Locate the original event even when presentation order or process grouping changes. */
export function findPreviewRowIndex(rows: readonly ConversationPreviewRow[], eventIndex: number): number {
  return rows.findIndex((row) => row.type === "event"
    ? row.event.index === eventIndex
    : row.events.some((event) => event.index === eventIndex));
}

/** Bound the cost of expanding one long process turn inside the virtual list. */
export function boundPreviewProcessRows(rows: readonly ConversationPreviewRow[]): ConversationPreviewRow[] {
  return rows.flatMap((row) => {
    if (row.type !== "process" || row.events.length <= 40) return [row];
    const chunks: ConversationPreviewRow[] = [];
    for (let offset = 0; offset < row.events.length; offset += 40) {
      const events = row.events.slice(offset, offset + 40);
      chunks.push({ ...row, key: events[0].index, events });
    }
    return chunks;
  });
}
