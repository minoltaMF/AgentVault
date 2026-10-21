export type PreviewSearchGap = { indices: number[] };

/** Eligible indices are loaded, filtered events in chronological order. */
export function previewSearchGaps(
  eligibleIndices: readonly number[],
  visibleIndices: readonly number[],
  newestFirst: boolean,
): Map<number, PreviewSearchGap> {
  const gaps = new Map<number, PreviewSearchGap>();
  const visible = new Set(visibleIndices);
  const ordered = newestFirst ? [...eligibleIndices].reverse() : eligibleIndices;
  let previous = -1;
  for (let position = 0; position < ordered.length; position++) {
    const index = ordered[position];
    if (!visible.has(index)) continue;
    if (previous >= 0 && position > previous + 1) {
      gaps.set(index, { indices: ordered.slice(previous + 1, position) });
    }
    previous = position;
  }
  return gaps;
}

/** Expand from the displayed leading edge without changing the gap's source data. */
export function gapExpansionIndices(gap: PreviewSearchGap, limit = 20): number[] {
  return gap.indices.slice(0, Math.max(0, Math.floor(limit)));
}
