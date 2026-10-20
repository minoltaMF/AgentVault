/** Indices identify source events; neighbors follow chronology, not index arithmetic. */
export function previewSearchContext(
  indices: readonly number[], active: number | null, before: number, after: number,
) {
  const position = active === null ? -1 : indices.indexOf(active);
  if (position < 0) return { indices: [] as number[], before: 0, after: 0, remainingBefore: 0, remainingAfter: 0 };
  const start = Math.max(0, position - Math.max(0, before));
  const end = Math.min(indices.length, position + Math.max(0, after) + 1);
  return {
    indices: indices.slice(start, end),
    before: position - start,
    after: end - position - 1,
    remainingBefore: start,
    remainingAfter: indices.length - end,
  };
}
