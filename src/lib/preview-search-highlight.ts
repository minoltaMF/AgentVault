export interface PreviewSearchRange { start: number; end: number }

/** Literal, non-overlapping matches expressed in the original UTF-16 offsets. */
export function findPreviewSearchRanges(text: string, query: string): PreviewSearchRange[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  const lower = text.toLowerCase();
  const found: PreviewSearchRange[] = [];
  let offset = lower.indexOf(needle);
  if (offset < 0) return found;
  if (lower.length === text.length) {
    while (offset >= 0) {
      found.push({start: offset, end: offset + needle.length});
      offset = lower.indexOf(needle, offset + needle.length);
    }
    return found;
  }
  // Lowercasing can expand a code point (İ -> i + combining dot). Map both
  // ends back to complete original code points rather than slicing by lower offsets.
  const starts: number[] = [];
  const ends: number[] = [];
  let original = 0;
  for (const point of text) {
    const end = original + point.length;
    for (let i = 0; i < point.toLowerCase().length; i++) {
      starts.push(original);
      ends.push(end);
    }
    original = end;
  }
  while (offset >= 0) {
    const range = {start: starts[offset], end: ends[offset + needle.length - 1]};
    const previous = found[found.length - 1];
    if (previous && range.start < previous.end) previous.end = Math.max(previous.end, range.end);
    else found.push(range);
    offset = lower.indexOf(needle, offset + needle.length);
  }
  return found;
}

interface HighlightNode {
  type: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HighlightNode[];
}
const blocks = new Set(["p", "div", "section", "article", "header", "footer", "main", "aside", "nav", "blockquote", "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "table", "thead", "tbody", "tfoot", "tr", "td", "th", "hr", "dl", "dt", "dd"]);
const excluded = new Set(["script", "style", "textarea", "mark"]);
const boundaries = new Set(["br", "img", "input", "hr"]);

/** Rehype attacher: highlight rendered text, retaining Markdown structure and attributes. */
export function rehypeSearchHighlight(options: { query: string; scope?: string }) {
  return (tree: HighlightNode): void => {
    if (!options.query.trim()) return;
    let occurrence = 0;
    let run: {node: HighlightNode; parent: HighlightNode; start: number; end: number}[] = [];
    let value = "";
    const flush = () => {
      const ranges = findPreviewSearchRanges(value, options.query);
      const occurrenceIds = ranges.map(() => `${options.scope ?? "search"}:${occurrence++}`);
      let rangeIndex = 0;
      for (const entry of run) {
        const source = entry.node.value ?? "";
        const replacements: HighlightNode[] = [];
        let cursor = 0;
        while (rangeIndex < ranges.length && ranges[rangeIndex].end <= entry.start) rangeIndex++;
        for (let i = rangeIndex; i < ranges.length && ranges[i].start < entry.end; i++) {
          const start = Math.max(0, ranges[i].start - entry.start);
          const end = Math.min(source.length, ranges[i].end - entry.start);
          if (start > cursor) replacements.push({type: "text", value: source.slice(cursor, start)});
          replacements.push({type: "element", tagName: "mark", properties: {className: ["preview-search-highlight"], "data-search-occurrence": occurrenceIds[i]}, children: [{type: "text", value: source.slice(start, end)}]});
          cursor = end;
        }
        if (!replacements.length) continue;
        if (cursor < source.length) replacements.push({type: "text", value: source.slice(cursor)});
        const index = entry.parent.children!.indexOf(entry.node);
        const children = entry.parent.children!;
        entry.parent.children = [...children.slice(0, index), ...replacements, ...children.slice(index + 1)];
      }
      run = [];
      value = "";
    };
    const visit = (node: HighlightNode, parent?: HighlightNode) => {
      const tag = node.tagName ?? "";
      if (excluded.has(tag) || boundaries.has(tag) || node.type === "raw") { flush(); return; }
      const block = blocks.has(tag);
      if (block) flush();
      if (node.type === "text" && parent) {
        const start = value.length;
        value += node.value ?? "";
        run.push({node, parent, start, end: value.length});
      } else {
        for (const child of [...(node.children ?? [])]) visit(child, node);
      }
      if (block) flush();
    };
    visit(tree);
    flush();
  };
}
