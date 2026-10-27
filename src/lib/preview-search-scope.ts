import { contextPageRequest } from "./preview-context-page";

export function previewSearchScope(start: number, end: number, tailReached: boolean, failed: boolean, busy: boolean, hits: number, pageSize: number) {
  const before = contextPageRequest("before", 0, start, end, tailReached, pageSize);
  const after = contextPageRequest("after", 0, start, end, tailReached, pageSize);
  const complete = start === 0 && tailReached && !failed && !busy;
  const range = end > start ? `已搜索原始事件 ${start + 1}–${end}` : "尚无已搜索事件";
  const result = busy ? "正在更新搜索范围…" : failed ? "读取中断，不能确认整个会话的搜索结果" : complete
    ? (hits ? "整个会话已搜索" : "整个会话无命中")
    : (hits ? "仅为已加载范围的命中" : "已加载范围无命中，仍有未读取内容");
  return { before, after, complete, range, result };
}
