import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { useLocation, useSearchParams } from "react-router-dom";
import { useSettings } from "@/stores/settings";
import { useWorkbenchSource } from "@/hooks/useWorkbenchSource";
import { workbenchSessions } from "@/lib/allSessions";
import { workbenchCache } from "@/lib/workbenchCache";
import { providerLabel } from "@/lib/providerTheme";
import { sessionIdentity } from "@/lib/sessionIdentity";
import type { SessionSummary } from "@/lib/api";
import { absoluteTime } from "@/lib/format";
import { WorkbenchSources } from "@/components/WorkbenchSources";
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
  return <Workbench key={JSON.stringify([settings.codex_dir, settings.claude_dir, settings.qoder_dir, settings.workbuddy_dir, settings.grok_dir, settings.pi_dir, settings.dsh_dir, settings.hermes_dir, settings.opencode_dir, settings.zcode_dir, settings.qwen_dir, settings.cline_dir, settings.copilot_dir, settings.antigravity_dir])} codexRoot={settings.codex_dir} claudeRoot={settings.claude_dir} qoderRoot={settings.qoder_dir ?? ""} workbuddyRoot={settings.workbuddy_dir ?? ""} grokRoot={settings.grok_dir ?? ""} piRoot={settings.pi_dir ?? ""} dshRoot={settings.dsh_dir ?? ""} hermesRoot={settings.hermes_dir ?? ""} opencodeRoot={settings.opencode_dir ?? ""} zcodeRoot={settings.zcode_dir ?? ""} qwenRoot={settings.qwen_dir ?? ""} clineRoot={settings.cline_dir ?? ""} copilotRoot={settings.copilot_dir ?? ""} antigravityRoot={settings.antigravity_dir ?? ""} />;
}

function Workbench({ codexRoot, claudeRoot, qoderRoot, workbuddyRoot, grokRoot, piRoot, dshRoot, hermesRoot, opencodeRoot, zcodeRoot, qwenRoot, clineRoot, copilotRoot, antigravityRoot }: { codexRoot: string; claudeRoot: string; qoderRoot: string; workbuddyRoot: string; grokRoot: string; piRoot: string; dshRoot: string; hermesRoot: string; opencodeRoot: string; zcodeRoot: string; qwenRoot: string; clineRoot: string; copilotRoot: string; antigravityRoot: string }) {
  const codex = useWorkbenchSource("codex", codexRoot, codexRoot);
  const claude = useWorkbenchSource("claude", claudeRoot, codexRoot);
  const qoder = useWorkbenchSource("qoder", qoderRoot, codexRoot);
  const workbuddy = useWorkbenchSource("workbuddy", workbuddyRoot, codexRoot);
  const grok = useWorkbenchSource("grok", grokRoot, codexRoot);
  const dsh = useWorkbenchSource("dsh", dshRoot, codexRoot);
  const hermes = useWorkbenchSource("hermes", hermesRoot, codexRoot);
  const opencode = useWorkbenchSource("opencode", opencodeRoot, codexRoot);
  const zcode = useWorkbenchSource("zcode", zcodeRoot, codexRoot);
  const qwen = useWorkbenchSource("qwen", qwenRoot, codexRoot);
  const cline = useWorkbenchSource("cline", clineRoot, codexRoot);
  const copilot = useWorkbenchSource("copilot", copilotRoot, codexRoot);
  const antigravity = useWorkbenchSource("antigravity", antigravityRoot, codexRoot);
  const pi = useWorkbenchSource("pi", piRoot, codexRoot);
  const [savedView] = useState(() => workbenchCache.view(codexRoot, claudeRoot, qoderRoot, workbuddyRoot, grokRoot, piRoot, dshRoot, hermesRoot, opencodeRoot, zcodeRoot, qwenRoot, clineRoot, copilotRoot, antigravityRoot));
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
  const provider = ["codex", "claude", "qoder", "workbuddy", "grok", "pi", "dsh", "hermes", "opencode", "zcode", "qwen", "cline", "copilot", "antigravity"].includes(params.get("provider") ?? "") ? params.get("provider")! : "";
  const project = params.get("project") ?? "";
  const archive = ["active", "archived"].includes(params.get("archive") ?? "") ? params.get("archive")! : "";
  const all = useMemo(() => [...codex.sessions, ...claude.sessions, ...qoder.sessions, ...workbuddy.sessions, ...grok.sessions, ...pi.sessions, ...dsh.sessions, ...hermes.sessions, ...opencode.sessions, ...zcode.sessions, ...qwen.sessions, ...cline.sessions, ...copilot.sessions, ...antigravity.sessions], [codex.sessions, claude.sessions, qoder.sessions, workbuddy.sessions, grok.sessions, pi.sessions, dsh.sessions, hermes.sessions, opencode.sessions, zcode.sessions, qwen.sessions, cline.sessions, copilot.sessions, antigravity.sessions]);
  const filtered = useMemo(() => workbenchSessions(all, { query, provider, project, archive }), [all, query, provider, project, archive]);
  const contentScopes = useMemo(() => workbenchSearchScopes(filtered), [filtered]);
  const contentScopeKey = JSON.stringify([codexRoot, claudeRoot, qoderRoot, workbuddyRoot, grokRoot, piRoot, dshRoot, hermesRoot, opencodeRoot, zcodeRoot, qwenRoot, clineRoot, copilotRoot, antigravityRoot, query, provider, project, archive, contentScopes]);
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
  const busy = codex.state === "loading" || claude.state === "loading" || qoder.state === "loading" || workbuddy.state === "loading" || grok.state === "loading" || pi.state === "loading" || dsh.state === "loading" || hermes.state === "loading" || opencode.state === "loading" || zcode.state === "loading" || qwen.state === "loading" || cline.state === "loading" || copilot.state === "loading" || antigravity.state === "loading";
  const sources = [{ name: "Codex", root: codexRoot, data: codex }, { name: "Claude", root: claudeRoot, data: claude }, { name: "Qoder CLI", root: qoderRoot, data: qoder }, { name: "WorkBuddy", root: workbuddyRoot, data: workbuddy }, { name: "Grok Build CLI", root: grokRoot, data: grok }, { name: "Pi", root: piRoot, data: pi },{name:"DeepSeek Harness",root:dshRoot,data:dsh},{name:"Hermes Agent",root:hermesRoot,data:hermes},{name:"OpenCode",root:opencodeRoot,data:opencode},{name:"ZCode",root:zcodeRoot,data:zcode},{name:"Qwen Code",root:qwenRoot,data:qwen}, {name:"Cline CLI/Desktop",root:clineRoot,data:cline}, {name:"GitHub Copilot CLI",root:copilotRoot,data:copilot}, {name:"Antigravity",root:antigravityRoot,data:antigravity}];
  const filter = (name: string, value: string) => {
    setParams((old) => { const next = new URLSearchParams(old); if (value) next.set(name, value); else next.delete(name); return next; }, { replace: true });
    setPage(0);
  };
  const refresh = () => { for (const source of sources) if (source.root) void source.data.refresh(); };

  return <>
    <TopBar title="全部会话" stats={`${filtered.length} 条`} refreshing={busy} onRefresh={refresh} showListTools={false} />
    <ScrollArea className="min-h-0 flex-1" viewportRef={viewport}>
      <div className="space-y-5 p-4 md:p-6">
        <p className="text-sm text-muted-foreground">在一处查找已支持来源的本地会话。选择读取来源后开始；预览为只读。</p>
        <WorkbenchSources sources={sources} view={savedView} />
        <div className="grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <label className="min-w-0 space-y-1 text-xs">搜索标题、首条消息、ID 或路径<Input aria-label="搜索会话" value={query} onChange={(e) => filter("q", e.target.value)} placeholder="不搜索全部正文" /></label>
          <label className="min-w-0 space-y-1 text-xs">来源<select aria-label="来源" className={selectClass} value={provider} onChange={(e) => filter("provider", e.target.value)}><option value="">所有来源</option><option value="codex">Codex</option><option value="claude">Claude</option><option value="qoder">Qoder CLI</option><option value="workbuddy">WorkBuddy</option><option value="grok">Grok Build CLI</option><option value="pi">Pi</option><option value="dsh">DeepSeek Harness</option><option value="hermes">Hermes Agent</option><option value="opencode">OpenCode</option><option value="zcode">ZCode</option><option value="qwen">Qwen Code</option><option value="cline">Cline CLI/Desktop</option><option value="copilot">GitHub Copilot CLI</option><option value="antigravity">Antigravity</option></select></label>
          <label className="min-w-0 space-y-1 text-xs">项目路径<select aria-label="项目路径" className={selectClass} value={project} onChange={(e) => filter("project", e.target.value)}><option value="">所有项目</option>{project && !projects.includes(project) && <option value={project}>{project}（尚无结果）</option>}{projects.map((cwd) => <option key={cwd} value={cwd}>{cwd}</option>)}</select></label>
          <label className="min-w-0 space-y-1 text-xs">归档状态<select aria-label="归档状态" className={selectClass} value={archive} onChange={(e) => filter("archive", e.target.value)}><option value="">全部状态</option><option value="active">未归档</option><option value="archived">已归档</option></select></label>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground"><span>按更新时间降序 · 共 {filtered.length} 条 · 每页 {PAGE_SIZE} 条</span>{(query || provider || project || archive) && <Button size="sm" variant="ghost" onClick={() => { setParams({}); setPage(0); }}>清除筛选</Button>}</div>
        <div className="flex flex-wrap items-center gap-3"><Button variant="outline" disabled={!filtered.length || busy} onClick={() => setContentSearchOpen(true)}>搜索正文</Button><span className="text-xs text-muted-foreground">手动搜索当前筛选范围内全部会话的用户与助手消息，不限于当前页。</span></div>
        {!filtered.length && <p role="status" className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">{busy ? "正在读取来源，请稍候…" : all.length ? "没有符合筛选条件的会话。" : sources.every((s) => s.data.state === "idle") ? "展开未使用来源选择读取，或使用顶部刷新读取已配置的来源。" : "当前没有可显示的会话，请检查来源状态或重试。"}</p>}
        <section aria-label="统一会话列表" className="space-y-2">
          {filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE).map((session) => <article key={sessionIdentity(session)} className="min-w-0 rounded-lg border p-4">
            <div className="flex flex-wrap items-center gap-2"><Badge variant="outline">{providerLabel(session.provider)}</Badge>{session.archived && <Badge variant="secondary">已归档</Badge>}<span className="ml-auto text-xs text-muted-foreground">{absoluteTime(session.updated_at)}</span></div>
            <Button variant="link" className="mt-1 h-auto max-w-full whitespace-normal break-all px-0 text-left" onClick={(event) => { previewButton.current = event.currentTarget; setPreview(session); setPreviewOpen(true); }}>{session.title || session.first_user_message || "未命名会话"}</Button>
            <p className="line-clamp-2 break-all text-sm text-muted-foreground">{session.first_user_message}</p>
            <p className="mt-2 break-all text-xs text-muted-foreground">项目：{session.cwd || "未记录项目路径"}</p>
            <p className="mt-1 break-all font-mono text-xs text-muted-foreground">ID：{session.id}</p>
          </article>)}
        </section>
        {pages > 1 && <nav aria-label="会话分页" className="flex items-center justify-center gap-3"><Button variant="outline" disabled={currentPage === 0} onClick={() => { setPage(currentPage - 1); viewport.current?.scrollTo({ top: 0 }); }}>上一页</Button><span className="text-sm">{currentPage + 1} / {pages}</span><Button variant="outline" disabled={currentPage + 1 >= pages} onClick={() => { setPage(currentPage + 1); viewport.current?.scrollTo({ top: 0 }); }}>下一页</Button></nav>}
      </div>
    </ScrollArea>
    <ContentSearchDialog key={contentScopeKey} open={contentSearchOpen} onOpenChange={setContentSearchOpen} provider="codex" codexDir={codexRoot} claudeDir={claudeRoot} qoderDir={qoderRoot} workbuddyDir={workbuddyRoot} grokDir={grokRoot} piDir={piRoot} dshDir={dshRoot} hermesDir={hermesRoot} zcodeDir={zcodeRoot} qwenDir={qwenRoot} clineDir={clineRoot} copilotDir={copilotRoot} antigravityDir={antigravityRoot} opencodeDir={opencodeRoot} cursorDir="" showSubagentSessions={false} showArchivedSessions={archive === "archived"} rolloutPaths={[]} workbenchScopes={contentScopes} retained={retainedContent} onOpenResult={(session, match, text) => { setPreviewJump({ eventIndex: match.event_index, eventOffset: match.event_offset, query: text }); setPreview(session); setPreviewOpen(true); }} />
    {preview && <PreviewDialog key={`${sessionIdentity(preview)}:${previewJump?.eventIndex ?? ""}`} readOnly open={previewOpen} session={preview} initialJump={previewJump} allSessions={all.filter((s) => s.provider === preview.provider)} onOpenChange={(open) => { if (!open) { setPreviewOpen(false); if (previewJump) { setPreviewJump(null); setContentSearchOpen(true); } else requestAnimationFrame(() => previewButton.current?.focus({ preventScroll: true })); } }} />}
  </>;
}
