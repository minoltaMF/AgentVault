import { useEffect, useMemo, useState } from "react";
import { api } from "@/lib/api";
import { aggregateUsage, estimateCost, parseRate, priceKey, priceStorageKey, rateFields, readPriceBook, summarizeUsage, type PriceBook, type UsageScope, type UsageSession, type UsageStatus } from "@/lib/session-usage";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
const labels = { input: "未缓存输入", output: "输出", cache_read: "缓存读取", cache_write_5m: "缓存写入（5 分钟）", cache_write_1h: "缓存写入（1 小时）" };
const money = (value: number) => `$${value.toLocaleString("en-US", { maximumFractionDigits: 6 })}`;
const count = (value: number) => value.toLocaleString();
const EMPTY_RESULTS: UsageSession[] = [];
function Totals({ sessions, prices }: { sessions: UsageSession[]; prices: PriceBook }) {
  const summary = useMemo(() => summarizeUsage(sessions, prices), [sessions, prices]);
  if (!sessions.length) return <p className="text-xs text-muted-foreground">尚无可汇总的会话用量，费用未知。</p>;
  if (summary.overflow) return <p role="alert" className="text-xs text-destructive">用量超出安全计算范围，无法提供准确汇总或费用。</p>;
  return <div className="space-y-1 text-xs"><p>未缓存输入 {count(summary.tokens.input)} · 输出 {count(summary.tokens.output)} · 缓存读取 {count(summary.tokens.cache_read)} · 缓存写入 {count(summary.tokens.cache_write)}</p><p>其中：1 小时缓存写入 {count(summary.tokens.cache_write_1h)} · 推理 {count(summary.tokens.reasoning)}（已计入输出）</p><p>{summary.unknownModels || summary.missingUsage || summary.warnedSessions ? "已知部分估算" : "估算费用"}：{summary.known === 0 && (summary.unknownModels || summary.missingUsage) ? "费用未知（已知部分为 0）" : `${money(summary.known)} USD`}{summary.unknownModels > 0 && ` · ${summary.unknownModels} 项模型用量缺少单价`}{summary.missingUsage > 0 && ` · ${summary.missingUsage} 条会话未记录用量`}</p>{summary.warnedSessions > 0 && <p className="text-amber-700 dark:text-amber-400">{summary.warnedSessions} 条会话存在读取或统计提示，结果可能不完整，请查看会话明细。</p>}</div>;
}
export function SessionUsageDialog({ open, onOpenChange, sessions, codexDir, claudeDir }: { open: boolean; onOpenChange: (open: boolean) => void; sessions: UsageScope[]; codexDir: string; claudeDir: string }) {
  const scope = useMemo(() => sessions.filter(session => session.provider === "codex" || session.provider === "claude"), [sessions]);
  const [status, setStatus] = useState<UsageStatus | null>(null);
  const [error, setError] = useState("");
  const [job, setJob] = useState<number | null>(null);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [view, setView] = useState<"session" | "model" | "project">("session");
  const [page, setPage] = useState(0);
  const [failurePage, setFailurePage] = useState(0);
  const [run, setRun] = useState({ sequence: 0, forceRefresh: false });
  const [prices, setPrices] = useState<PriceBook>(() => { try { return readPriceBook(localStorage.getItem(priceStorageKey)); } catch { return {}; } });
  const [priceError, setPriceError] = useState("");
  const [editing, setEditing] = useState("");
  const [draft, setDraft] = useState<Record<string, string>>({});
  useEffect(() => {
    if (!open || !scope.length) return;
    let disposed = false, id: number | null = null, timer: ReturnType<typeof setTimeout> | undefined;
    setStatus(null); setError(""); setJob(null); setCancelRequested(false); setFailurePage(0);
    async function poll() {
      try {
        const next = await api.sessionUsageStatus(id!);
        if (disposed) return;
        // Progress-only polls should not recompute thousands of unchanged model totals.
        setStatus(previous => previous?.processed_files === next.processed_files && previous.state === next.state && previous.error === next.error ? { ...previous, elapsed_ms: next.elapsed_ms } : next);
        if (next.state === "running") timer = setTimeout(() => void poll(), 400);
      } catch (failure) { if (!disposed) setError(String(failure)); }
    }
    void api.startSessionUsage(scope, codexDir, claudeDir, run.forceRefresh).then(result => {
      id = result.job_id;
      if (disposed) { void api.cancelSessionUsage(id).catch(() => {}); return; }
      setJob(id); void poll();
    }).catch(failure => { if (!disposed) setError(String(failure)); });
    return () => { disposed = true; clearTimeout(timer); if (id !== null) void api.cancelSessionUsage(id).catch(() => {}); };
  }, [open, scope, codexDir, claudeDir, run]);
  const results = status?.results ?? EMPTY_RESULTS;
  const models = useMemo(() => [...new Map(results.flatMap(session => session.models.map(model => [priceKey(session.provider, model.model), `${session.provider} / ${model.model || "未知模型"}`] as const))).entries()], [status?.results]);
  const groups = useMemo(() => view === "session" ? results.map(session => ({ label: session.title || session.id, sessions: [session] })) : aggregateUsage(results, view), [status?.results, view]);
  const pages = Math.max(1, Math.ceil(groups.length / 20));
  const current = Math.min(page, pages - 1);
  const busy = !error && (!status || status.state === "running") && scope.length > 0;
  function edit(key: string) { setEditing(key); setDraft(Object.fromEntries(rateFields.map(field => [field, prices[key]?.[field]?.toString() ?? ""]))); setPriceError(""); }
  function save() {
    try {
      const next = { ...prices, [editing]: Object.fromEntries(rateFields.map(field => [field, parseRate(draft[field] ?? "")])) };
      localStorage.setItem(priceStorageKey, JSON.stringify({ version: 1, prices: next }));
      setPrices(next); setEditing(""); setPriceError("");
    } catch (failure) { setPriceError(`无法保存单价：${String(failure)}`); }
  }
  function changeOpen(nextOpen: boolean) {
    if (!nextOpen) setRun(previous => previous.forceRefresh ? { ...previous, forceRefresh: false } : previous);
    onOpenChange(nextOpen);
  }
  return <Dialog open={open} onOpenChange={changeOpen}><DialogContent className="flex max-h-[90vh] max-w-4xl flex-col overflow-hidden"><DialogHeader><DialogTitle>会话用量与成本</DialogTitle><DialogDescription>分析当前筛选的全部 {scope.length} 条 Codex / Claude 会话，不限当前页；跳过其他来源 {sessions.length - scope.length} 条。只读取原生记录。未变化文件复用本次运行的缓存，变化文件重新解析；重启应用后缓存清空。</DialogDescription></DialogHeader>
    <div className="min-h-0 space-y-4 overflow-y-auto pr-2">
      <div role="status" className="flex flex-wrap items-center gap-3 text-sm"><span>{status ? `${status.processed_files} / ${status.total_files} 个文件 · ${status.state === "running" ? "分析中" : status.state === "completed" ? "已完成" : status.state === "cancelled" ? "已取消，以下为部分结果" : "失败，以下为部分结果"}` : scope.length ? "正在启动分析…" : "当前范围没有支持的来源。"}</span>{busy && <Button size="sm" variant="outline" disabled={job === null || cancelRequested} onClick={() => { setCancelRequested(true); void api.cancelSessionUsage(job!).catch(failure => { setError(String(failure)); setCancelRequested(false); }); }}>{cancelRequested ? "正在取消…" : "取消分析"}</Button>}</div>
      {!busy && scope.length > 0 && <div className="flex flex-wrap items-center gap-2"><Button size="sm" variant="outline" onClick={() => setRun(previous => ({ sequence: previous.sequence + 1, forceRefresh: true }))}>强制重新分析</Button><span className="text-xs text-muted-foreground">绕过缓存，重新读取当前筛选范围。</span></div>}
      {status && <p className="text-xs text-muted-foreground">已复用 {status.cache_hits} 个文件 · 已重新解析 {status.parsed_files} 个文件 · 用时 {(status.elapsed_ms / 1000).toFixed(2)} 秒{run.forceRefresh ? " · 本次已绕过缓存" : ""}</p>}
      {status && <progress className="w-full" aria-label="用量分析进度" value={status.processed_files} max={Math.max(1, status.total_files)} />}
      {(error || status?.error) && <p role="alert" className="break-all text-sm text-destructive">{error || status?.error}</p>}
      {status && <div className="rounded-md border p-3"><p className="mb-2 text-sm font-medium">已成功读取 {results.length} 条会话{busy ? "（分析尚未完成）" : status.failures.length || status.state !== "completed" ? "（部分结果）" : ""}</p><Totals sessions={results} prices={prices} /></div>}
      {!!status?.failures.length && <details className="rounded-md border p-3"><summary className="cursor-pointer text-sm text-destructive">{status.failures.length} 个文件分析失败（不计入汇总）</summary>{status.failures.slice(failurePage * 20, (failurePage + 1) * 20).map(failure => <p key={failure.path} className="mt-2 break-all text-xs">{failure.path}：{failure.error}</p>)}<div className="mt-2 flex gap-2"><Button size="sm" variant="outline" disabled={!failurePage} onClick={() => setFailurePage(failurePage - 1)}>上一页失败</Button><Button size="sm" variant="outline" disabled={(failurePage + 1) * 20 >= status.failures.length} onClick={() => setFailurePage(failurePage + 1)}>下一页失败</Button></div></details>}
      {!!models.length && <details className="rounded-md border p-3"><summary className="cursor-pointer text-sm">模型单价（用户录入，USD / 百万 Token）</summary><p className="my-2 text-xs text-muted-foreground">留空表示未知，0 表示免费。仅保存在本机；估算不等于账单，不含套餐、折扣和税费。</p><select aria-label="选择定价模型" className="h-9 w-full rounded-md border bg-background px-2 text-sm" value={editing} onChange={event => edit(event.target.value)}><option value="">选择模型以编辑单价</option>{models.map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select>{editing && <div className="mt-3 grid gap-3 sm:grid-cols-2">{rateFields.map(field => <label key={field} className="space-y-1 text-xs">{labels[field]}<Input inputMode="decimal" aria-label={`${labels[field]}单价`} value={draft[field] ?? ""} placeholder="未知" onChange={event => setDraft(previous => ({ ...previous, [field]: event.target.value }))} /></label>)}<Button onClick={save}>保存单价</Button></div>}{priceError && <p role="alert" className="mt-2 text-xs text-destructive">{priceError}</p>}</details>}
      <div className="flex flex-wrap gap-2" aria-label="用量分组">{(["session", "model", "project"] as const).map(value => <Button key={value} size="sm" variant={view === value ? "default" : "outline"} onClick={() => { setView(value); setPage(0); }}>{value === "session" ? "按会话" : value === "model" ? "按模型" : "按项目"}</Button>)}</div>
      {groups.slice(current * 20, (current + 1) * 20).map((group, index) => <article key={`${view}:${current}:${index}`} className="space-y-2 rounded-md border p-3"><h3 className="break-all text-sm font-medium">{view === "model" ? (JSON.parse(group.label) as string[]).join(" / ") : group.label}</h3><Totals sessions={group.sessions} prices={prices} />{view === "session" && <><p className="break-all text-xs text-muted-foreground">{group.sessions[0].provider} · {group.sessions[0].cwd || "未记录项目路径"}</p><details><summary className="cursor-pointer text-xs">模型明细与读取提示</summary>{group.sessions[0].models.map(model => { const cost = estimateCost(model.tokens, prices[priceKey(group.sessions[0].provider, model.model)]); return <div key={model.model} className="mt-2 text-xs"><p>{model.model || "未知模型"}：{cost.unknown.length ? "已知部分 " : ""}{money(cost.known)}{cost.unknown.length ? `；缺少 ${cost.unknown.map(field => labels[field]).join("、")} 单价` : ""}</p><Totals sessions={[{ ...group.sessions[0], models: [model] }]} prices={prices} /></div>; })}{group.sessions[0].warnings.map((warning, warningIndex) => <p key={warningIndex} className="mt-2 break-all text-xs text-amber-700 dark:text-amber-400">{warning}</p>)}<p className="mt-2 break-all text-xs text-muted-foreground">{group.sessions[0].rollout_path}</p></details></>}</article>)}
      {groups.length > 20 && <nav aria-label="用量分页" className="flex items-center justify-center gap-3"><Button variant="outline" disabled={!current} onClick={() => setPage(current - 1)}>上一页</Button><span className="text-sm">{current + 1} / {pages}</span><Button variant="outline" disabled={current + 1 >= pages} onClick={() => setPage(current + 1)}>下一页</Button></nav>}
    </div>
  </DialogContent></Dialog>;
}
