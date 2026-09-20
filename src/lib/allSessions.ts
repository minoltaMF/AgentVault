import type { SessionSummary, WorkbenchScanStatus } from "./api";
import { sessionIdentity } from "./sessionIdentity";

export type WorkbenchProvider = "codex" | "claude" | "qoder";
export type WorkbenchFilters = { query: string; provider: string; project: string; archive: string };

export function scanProgressText(scan: WorkbenchScanStatus): string {
  const phase = ({ discovering: "发现文件", reading_index: "读取索引", reading: "读取文件", annotating: "整理来源信息", finished: "扫描结束" } as Record<string, string>)[scan.phase] ?? "扫描";
  return `${phase}：已发现 ${scan.discovered_files} 个文件，已处理 ${scan.processed_files} 个，失败 ${scan.failed_files} 项`;
}

export function workbenchSessions(sessions: readonly SessionSummary[], filters: WorkbenchFilters) {
  const query = filters.query.trim().toLocaleLowerCase();
  return sessions.filter((item) =>
    (!filters.provider || item.provider === filters.provider)
    && (!filters.project || item.cwd === filters.project)
    && (!filters.archive || item.archived === (filters.archive === "archived"))
    && (!query || [item.title, item.first_user_message, item.id, item.cwd].some((text) => text.toLocaleLowerCase().includes(query)))
  ).sort((a, b) => b.updated_at - a.updated_at || sessionIdentity(a).localeCompare(sessionIdentity(b)));
}

/** Invalidated requests may finish, but cannot publish into another source or retry. */
export function requestGeneration() {
  let generation = 0;
  return { next: () => ++generation, current: (value: number) => value === generation, invalidate: () => { generation++; } };
}
