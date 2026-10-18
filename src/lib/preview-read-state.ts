export type PreviewReadFailure = { kind: "unavailable" | "failed"; detail: string };

/** Only the backend's explicit prefix identifies an invalid source, not localized text. */
export function previewReadFailure(error: unknown): PreviewReadFailure {
  const message = error instanceof Error ? error.message : String(error);
  const unavailable = message.startsWith("[PREVIEW_FILE_UNAVAILABLE]");
  return {
    kind: unavailable ? "unavailable" : "failed",
    detail: message.replace(/^\[PREVIEW_(FILE_UNAVAILABLE|READ_FAILED)\]\s*/, ""),
  };
}

export function previewEmptyMessage(eventCount: number, startOffset: number, filtered: boolean): string {
  if (eventCount > 0 || filtered) return "当前筛选下没有匹配消息";
  return startOffset > 0 ? "当前页没有可显示事件，请返回最早位置重新读取" : "空会话：未读取到正文事件";
}
