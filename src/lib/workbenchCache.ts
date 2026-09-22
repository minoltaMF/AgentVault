import type { WorkbenchScanStatus } from "./api";
import type { WorkbenchProvider } from "./allSessions";

export type SourceCache = { completed: { progress: WorkbenchScanStatus; checkedAt: string } | null };
export type WorkbenchView = { search: string; page: number; scrollTop: number };

/** Application-lifetime only. Keep one scope per provider; abandoned scopes cannot publish
 * into their replacements, and changing back to an old root does not resurrect its data. */
export function createWorkbenchCache() {
  const sources = new Map<WorkbenchProvider, { scope: string; value: SourceCache }>();
  let view: { scope: string; value: WorkbenchView } | undefined;
  return {
    source(provider: WorkbenchProvider, root: string, codexRoot: string): SourceCache {
      const scope = JSON.stringify([root, provider !== "codex" ? codexRoot : ""]);
      const existing = sources.get(provider);
      if (existing?.scope === scope) return existing.value;
      const value: SourceCache = { completed: null };
      sources.set(provider, { scope, value });
      return value;
    },
    view(codexRoot: string, claudeRoot: string, qoderRoot = "", workbuddyRoot = "", grokRoot = "", piRoot = ""): WorkbenchView {
      const scope = JSON.stringify([codexRoot, claudeRoot, qoderRoot, workbuddyRoot, grokRoot, piRoot]);
      if (view?.scope !== scope) view = { scope, value: { search: "", page: 0, scrollTop: 0 } };
      return view.value;
    },
  };
}

export const workbenchCache = createWorkbenchCache();
