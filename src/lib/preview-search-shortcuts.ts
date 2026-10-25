export type PreviewSearchShortcut = "focus" | "previous-message" | "next-message" | "previous-occurrence" | "next-occurrence";

type ShortcutEvent = {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
  isComposing?: boolean;
  keyCode?: number;
  defaultPrevented?: boolean;
};

/** Resolve only shortcuts owned by the current preview, never nested surfaces. */
export function previewSearchShortcut(event: ShortcutEvent, scope: { inPreview: boolean; inSearch: boolean; editing: boolean }): PreviewSearchShortcut | null {
  if (!scope.inPreview || event.defaultPrevented || event.isComposing || event.keyCode === 229) return null;
  const { ctrlKey, metaKey, altKey, shiftKey } = event;
  if (event.key.toLowerCase() === "f" && !!ctrlKey !== !!metaKey && !altKey && !shiftKey) return "focus";
  if (ctrlKey || metaKey) return null;
  if (event.key === "F3" && !altKey && (!scope.editing || scope.inSearch)) {
    return shiftKey ? "previous-message" : "next-message";
  }
  if (event.key === "Enter" && scope.inSearch) {
    if (altKey) return shiftKey ? "previous-occurrence" : "next-occurrence";
    return shiftKey ? "previous-message" : "next-message";
  }
  return null;
}
