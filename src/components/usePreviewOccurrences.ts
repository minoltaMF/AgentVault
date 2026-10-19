import { useEffect, useState, type RefObject } from "react";
import { clampOccurrenceIndex, collectPreviewOccurrences, stepOccurrenceIndex } from "@/lib/preview-search-occurrences";

export function usePreviewOccurrences(viewport: RefObject<HTMLDivElement>, eventIndex: number | null, query: string, open: boolean) {
  const key = JSON.stringify([query, eventIndex]);
  const [selection, setSelection] = useState({key: "", index: 0, total: 0});
  const index = selection.key === key ? selection.index : 0;
  const total = selection.key === key ? selection.total : 0;

  useEffect(() => {
    const root = viewport.current;
    if (!open || !query || eventIndex === null) {
      setSelection(previous => previous.key === key && previous.index === 0 && previous.total === 0
        ? previous : {key, index: 0, total: 0});
      return;
    }
    if (!root) return;
    let frame = 0;
    const update = () => {
      frame = 0;
      const event = root.querySelector<HTMLElement>(`[data-event-index="${eventIndex}"]`);
      // Retain the count while the selected virtual row is offscreen. Navigation
      // remounts it through the existing message jump path.
      if (!event) return;
      const groups = collectPreviewOccurrences(event);
      const selected = clampOccurrenceIndex(index, groups.length);
      root.querySelectorAll("mark.preview-search-current").forEach(mark => mark.classList.remove("preview-search-current"));
      groups[selected]?.forEach(mark => mark.classList.add("preview-search-current"));
      setSelection(previous => previous.key === key && previous.index === selected && previous.total === groups.length
        ? previous : {key, index: selected, total: groups.length});
    };
    const observer = new MutationObserver(() => {
      if (!frame) frame = requestAnimationFrame(update);
    });
    observer.observe(root, {childList: true, subtree: true});
    update();
    return () => {
      observer.disconnect();
      if (frame) cancelAnimationFrame(frame);
      root.querySelectorAll("mark.preview-search-current").forEach(mark => mark.classList.remove("preview-search-current"));
    };
  }, [eventIndex, index, key, open, query, viewport]);

  return {index, total, step: (direction: 1 | -1) => {
    setSelection({key, total, index: stepOccurrenceIndex(index, total, direction)});
  }};
}
