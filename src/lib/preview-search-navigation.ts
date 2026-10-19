/** Source indices are identities, not ordinal positions in the filtered list. */
export function selectPreviewSearchHit(hits: readonly number[], current: number | null, preferred: number | null): number | null {
  if (current !== null && hits.includes(current)) return current;
  if (preferred !== null && hits.includes(preferred)) return preferred;
  return hits[0] ?? null;
}
export function stepPreviewSearchHit(hits: readonly number[], current: number | null, direction: 1 | -1): number | null {
  if (!hits.length) return null;
  const index = current === null ? -1 : hits.indexOf(current);
  if (index < 0) return direction === 1 ? hits[0] : hits[hits.length - 1];
  return hits[(index + direction + hits.length) % hits.length];
}
export function previewSearchScope(start: number, end: number, total: number, newestFirst: boolean, done: boolean, failed: boolean): string {
  if (failed) return "部分结果 · 读取中断";
  return start === 0 && ((total > 0 && end >= total) || (!newestFirst && done)) ? "完整会话" : "已加载范围";
}
