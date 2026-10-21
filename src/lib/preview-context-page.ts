/** One click may read at most one page, even if all its events are filtered out. */
export function contextPageRequest(
  direction: "before" | "after", remaining: number, start: number, end: number,
  tailReached: boolean, pageSize: number,
): { start: number; count: number } | null {
  if (remaining >= 3) return null;
  if (direction === "before") {
    const previousStart = Math.max(0, start - pageSize);
    return start > 0 ? { start: previousStart, count: start - previousStart } : null;
  }
  return tailReached ? null : { start: end, count: pageSize };
}

export function previewTailReached(previous: boolean, replace: boolean, prepend: boolean, received: number, requested: number) {
  return !replace && prepend ? previous : received < requested;
}
