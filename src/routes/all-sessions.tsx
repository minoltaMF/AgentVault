import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useLocation, useSearchParams } from "react-router-dom";
import { useSettings } from "@/stores/settings";
import { useWorkbenchSource } from "@/hooks/useWorkbenchSource";
import { scanProgressText, workbenchSessions } from "@/lib/allSessions";
import { workbenchCache } from "@/lib/workbenchCache";
import { sessionIdentity } from "@/lib/sessionIdentity";
import type { SessionSummary } from "@/lib/api";
import { absoluteTime } from "@/lib/format";
import { TopBar } from "@/components/TopBar";
import { PreviewDialog, type PreviewJump } from "@/components/PreviewDialog";
import { ContentSearchDialog } from "@/components/ContentSearchDialog";
import { contentSearchCache, workbenchSearchScopes } from "@/lib/workbenchContentSearch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";

const PAGE_SIZE = 50;
const selectClass = "h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm";

export default function AllSessionsRoute() {
  const settings = useSettings((s) => s.settings);
  if (!settings) return null;
  return <Workbench key={JSON.stringify([settings.codex_dir, settings.claude_dir])} codexRoot={settings.codex_dir} claudeRoot={settings.claude_dir} />;
}

function Workbench({ codexRoot, claudeRoot }: { codexRoot: string; claudeRoot: string }) {
  const codex = useWorkbenchSource("codex", codexRoot, codexRoot);
  const claude = useWorkbenchSource("claude", claudeRoot, codexRoot);
  const [savedView] = useState(() => workbenchCache.view(codexRoot, claudeRoot));
  const location = useLocation();
  // An explicit URL is authoritative; ordinary sidebar navigation restores the last view.
  const [initialSearch] = useState(() => location.search ? location.search.slice(1) : savedView.search);
  const [params, setParams] = useSearchParams(initialSearch);
  const restoredUrl = useRef(false);
  useLayoutEffect(() => {
    if (restoredUrl.current) return;
    restoredUrl.current = true;
    if (!location.search && initialSearch) setParams(initialSearch, { replace: true });
  }, [initialSearch, location.search, setParams]);
  const [preview, setPreview] = useState<SessionSummary | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [contentSearchOpen, setContentSearchOpen] = useState(false);
  const [previewJump, setPreviewJump] = useState<PreviewJump | null>(null);
  const previewButton = useRef<HTMLButtonElement | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const [page, setPage] = useState(() => initialSearch === savedView.search ? savedView.page : 0);
  const [initialScroll] = useState(() => initialSearch === savedView.search ? savedView.scrollTop : 0);
  const query = params.get("q") ?? "";
  const provider = ["codex", "claude"].includes(params.get("provider") ?? "") ? params.get("provider")! : "";
  const project = params.get("project") ?? "";
  const archive = ["active", "archived"].includes(params.get("archive") ?? "") ? params.get("archive")! : "";
  const all = useMemo(() => [...codex.sessions, ...claude.sessions], [codex.sessions, claude.sessions]);
  const filtered = useMemo(() => workbenchSessions(all, { query, provider, project, archive }), [all, query, provider, project, archive]);
  const contentScopes = useMemo(() => workbenchSearchScopes(filtered), [filtered]);
  const contentScopeKey = JSON.stringify([codexRoot, claudeRoot, query, provider, project, archive, contentScopes]);
  const retainedContent = useMemo(() => contentSearchCache(contentScopeKey), [contentScopeKey]);
  const projects = useMemo(() => [...new Set(all.map((s) => s.cwd).filter(Boolean))].sort(), [all]);
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, pages - 1);
  const search = params.toString();
  useLayoutEffect(() => {
    const element = viewport.current;
    if (element) element.scrollTop = initialScroll;
    return () => { if (element) savedView.scrollTop = element.scrollTop; };
  }, [savedView, initialScroll]);
  useLayoutEffect(() => {
    savedView.search = search;
    savedView.page = currentPage;
  }, [savedView, search, currentPage]);
  const busy = codex.state === "loading" || claude.state === "loading";
  const sources = [{ name: "Codex", root: codexRoot, data: codex }, { name: "Claude", root: claudeRoot, data: claude }];
  const filter = (name: string, value: string) => {
    setParams((old) => { const next = new URLSearchParams(old); if (value) next.set(name, value); else next.delete(name); return next; }, { replace: true });
    setPage(0);
  };
  const refresh = () => { void codex.refresh(); void claude.refresh(); };

  return <>
    <TopBar title="全部会话" stats={`${filtered.length} 条`} refreshing={busy} onRefresh={refresh} showListTools={false} />
    <ScrollArea className="min-h-0 flex-1" viewportRef={viewport}>
      <div className="space-y-5 p-4 md:p-6">
        <p className="text-sm text-muted-foreground">在一处查找 Codex 与 Claude 会话。选择读取来源后开始；预览为只读。</p>
        <div className="grid min-w-0 gap-3 lg:grid-cols-2">
          {sources.map(({ name, root, data }) => <section key={name} aria-label={`${name} 来源`} className="min-w-0 space-y-2 rounded-lg border p-3">
            <div className="flex flex-wrap items-center justify-between gap-2"><h2 className="font-medium">{name}</h2><Badge variant="outline">{!root ? "未配置" : data.state === "interrupted" ? "状态待确认" : data.state === "cancelled" ? "已停止" : data.state === "loading" ? data.cancelling ? "正在停止" : "正在读取" : data.state === "error" ? "读取失败" : data.state === "ready" ? `${data.progress?.failed_files ? "部分完成 · " : ""}已读取 ${data.sessions.length} 条` : "尚未读取"}</Badge></div>
            <p className="break-all text-xs text-muted-foreground">设置中的来源：{root || "请先配置数据目录"}</p>
            {data.checkedAt && <p className="text-xs text-muted-foreground">上次扫描完成：{new Date(data.checkedAt).toLocaleString()}；显示上次结果，刷新后更新</p>}
            {data.error && <p role="alert" className="break-all text-xs text-destructive">{data.error}</p>}
            {data.progress && <div className="space-y-1 text-xs">
              <p role="status">{scanProgressText(data.progress)}</p>
              {data.progress.current_path && data.state === "loading" && <p className="truncate text-muted-foreground" title={data.progress.current_path}>当前：{data.progress.current_path}</p>}
              {!!data.progress.failed_files && <details><summary className="cursor-pointer text-destructive">查看文件 / 目录失败报告（{data.progress.failed_files}）</summary>
                <ul className="mt-2 max-h-48 space-y-2 overflow-auto">{data.progress.errors.map((item, index) => <li key={`${item.path}:${index}`} className="break-all rounded border p-2"><p className="font-mono">{item.path}</p><p className="mt-1">{item.message}</p></li>)}</ul>
                {data.progress.errors_truncated && <p className="mt-2 text-muted-foreground">报告过长，仅显示部分失败详情；总数仍计入上方统计。</p>}
              </details>}
            </div>}
            {data.state === "cancelled" && <p className="text-xs text-muted-foreground">本次扫描已停止，未用未完成的结果替换列表。可重新读取。</p>}
            {data.state === "interrupted" && <p className="text-xs text-muted-foreground">扫描可能仍在运行。请重试查询，或请求停止；此时不会创建重复任务。</p>}
            <div className="flex flex-wrap gap-2">
              <Button size="sm" variant="outline" disabled={!root || data.state === "loading" || data.state === "interrupted"} onClick={() => { void data.refresh(); }}>{data.state === "loading" ? "正在读取…" : data.state === "error" ? `重试 ${name}` : data.checkedAt ? `刷新 ${name}` : `读取 ${name}`}</Button>
              {(data.state === "loading" || data.state === "interrupted") && <Button size="sm" variant="outline" disabled={data.cancelling} onClick={() => { void data.cancel(); }}>{data.cancelling ? "正在停止…" : `停止 ${name} 扫描`}</Button>}
              {data.state === "interrupted" && <Button size="sm" variant="outline" onClick={data.retry}>重试查询 {name}</Button>}
            </div>
          </section>)}
        </div>
        <div className="grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <label className="min-w-0 space-y-1 text-xs">搜索标题、首条消息、ID 或路径<Input aria-label="搜索会话" value={query} onChange={(e) => filter("q", e.target.value)} placeholder="不搜索全部正文" /></label>
          <label className="min-w-0 space-y-1 text-xs">来源<select aria-label="来源" className={selectClass} value={provider} onChange={(e) => filter("provider", e.target.value)}><option value="">所有来源</option><option value="codex">Codex</option><option value="claude">Claude</option></select></label>
          <label className="min-w-0 space-y-1 text-xs">项目路径<select aria-label="项目路径" className={selectClass} value={project} onChange={(e) => filter("project", e.target.value)}><option value="">所有项目</option>{project && !projects.includes(project) && <option value={project}>{project}（尚无结果）</option>}{projects.map((cwd) => <option key={cwd} value={cwd}>{cwd}</option>)}</select></label>
          <label className="min-w-0 space-y-1 text-xs">归档状态<select aria-label="归档状态" className={selectClass} value={archive} onChange={(e) => filter("archive", e.target.value)}><option value="">全部状态</option><option value="active">未归档</option><option value="archived">已归档</option></select></label>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground"><span>按更新时间降序 · 共 {filtered.length} 条 · 每页 {PAGE_SIZE} 条</span>{(query || provider || project || archive) && <Button size="sm" variant="ghost" onClick={() => { setParams({}); setPage(0); }}>清除筛选</Button>}</div>
        <div className="flex flex-wrap items-center gap-3"><Button variant="outline" disabled={!filtered.length || busy} onClick={() => setContentSearchOpen(true)}>搜索正文</Button><span className="text-xs text-muted-foreground">手动搜索当前筛选范围内全部会话的用户与助手消息，不限于当前页。</span></div>
        {!filtered.length && <p role="status" className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">{busy ? "正在读取来源，请稍候…" : all.length ? "没有符合筛选条件的会话。" : sources.every((s) => s.data.state === "idle") ? "点击来源卡片读取，或使用顶部刷新读取已配置的两个来源。" : "当前没有可显示的会话，请检查来源状态或重试。"}</p>}
        <section aria-label="统一会话列表" className="space-y-2">
          {filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE).map((session) => <article key={sessionIdentity(session)} className="min-w-0 rounded-lg border p-4">
            <div className="flex flex-wrap items-center gap-2"><Badge variant="outline">{session.provider === "codex" ? "Codex" : "Claude"}</Badge>{session.archived && <Badge variant="secondary">已归档</Badge>}<span className="ml-auto text-xs text-muted-foreground">{absoluteTime(session.updated_at)}</span></div>
            <Button variant="link" className="mt-1 h-auto max-w-full whitespace-normal break-all px-0 text-left" onClick={(event) => { previewButton.current = event.currentTarget; setPreview(session); setPreviewOpen(true); }}>{session.title || session.first_user_message || "未命名会话"}</Button>
            <p className="line-clamp-2 break-all text-sm text-muted-foreground">{session.first_user_message}</p>
            <p className="mt-2 break-all text-xs text-muted-foreground">项目：{session.cwd || "未记录项目路径"}</p>
            <p className="mt-1 break-all font-mono text-xs text-muted-foreground">ID：{session.id}</p>
          </article>)}
        </section>
        {pages > 1 && <nav aria-label="会话分页" className="flex items-center justify-center gap-3"><Button variant="outline" disabled={currentPage === 0} onClick={() => { setPage(currentPage - 1); viewport.current?.scrollTo({ top: 0 }); }}>上一页</Button><span className="text-sm">{currentPage + 1} / {pages}</span><Button variant="outline" disabled={currentPage + 1 >= pages} onClick={() => { setPage(currentPage + 1); viewport.current?.scrollTo({ top: 0 }); }}>下一页</Button></nav>}
      </div>
    </ScrollArea>
    <ContentSearchDialog key={contentScopeKey} open={contentSearchOpen} onOpenChange={setContentSearchOpen} provider="codex" codexDir={codexRoot} claudeDir={claudeRoot} opencodeDir="" cursorDir="" showSubagentSessions={false} showArchivedSessions={archive === "archived"} rolloutPaths={[]} workbenchScopes={contentScopes} retained={retainedContent} onOpenResult={(session, match, text) => { setPreviewJump({ eventIndex: match.event_index, eventOffset: match.event_offset, query: text }); setPreview(session); setPreviewOpen(true); }} />
    {preview && <PreviewDialog key={`${sessionIdentity(preview)}:${previewJump?.eventIndex ?? ""}`} readOnly open={previewOpen} session={preview} initialJump={previewJump} allSessions={all.filter((s) => s.provider === preview.provider)} onOpenChange={(open) => { if (!open) { setPreviewOpen(false); if (previewJump) { setPreviewJump(null); setContentSearchOpen(true); } else requestAnimationFrame(() => previewButton.current?.focus({ preventScroll: true })); } }} />}
  </>;
}
