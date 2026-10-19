export function groupSearchMarks<T extends { getAttribute(name: string): string | null }>(marks: readonly T[]): T[][] {
  const groups: T[][] = [];
  const byId = new Map<string, T[]>();
  for (const mark of marks) {
    const id = mark.getAttribute("data-search-occurrence");
    const existing = id ? byId.get(id) : undefined;
    if (existing) existing.push(mark);
    else {
      const group = [mark];
      groups.push(group);
      if (id) byId.set(id, group);
    }
  }
  return groups;
}

export function collectPreviewOccurrences(root: ParentNode): HTMLElement[][] {
  // Disclosure summaries are repeated in their body; navigation counts body text.
  return groupSearchMarks([...root.querySelectorAll<HTMLElement>("mark.preview-search-highlight")]
    .filter(mark => !mark.closest("button")));
}

export function clampOccurrenceIndex(index: number, total: number): number {
  return total > 0 ? Math.max(0, Math.min(index, total - 1)) : 0;
}

export function stepOccurrenceIndex(index: number, total: number, direction: 1 | -1): number {
  return total > 0 ? (clampOccurrenceIndex(index, total) + direction + total) % total : 0;
}
