import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { summarizeUsage, type PriceBook, type UsageSession } from "@/lib/session-usage";
import { rankUsage, type UsageRankingGroup, type UsageRankingSort } from "@/lib/usage-ranking";

const money = (value: number) => `$${value.toLocaleString("en-US", { maximumFractionDigits: 6 })}`;
function Cost({ summary }: { summary: ReturnType<typeof summarizeUsage> }) {
  if (summary.overflow) return <span>无法可靠统计</span>;
  const partial = summary.unknownModels > 0 || summary.missingUsage > 0 || summary.warnedSessions > 0;
  return <span>{partial ? "已知部分 " : ""}{money(summary.known)}{partial && "（不完整）"}</span>;
}
export function UsageRankingPanel({ sessions, prices, complete, onOpenSession }: { sessions: UsageSession[]; prices: PriceBook; complete: boolean; onOpenSession: (session: UsageSession, trigger: HTMLButtonElement) => void }) {
  const [by, setBy] = useState<UsageRankingGroup>("model");
  const [sort, setSort] = useState<UsageRankingSort>("tokens");
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [sessionPage, setSessionPage] = useState(0);
  const rows = useMemo(() => rankUsage(sessions, prices, by, sort), [sessions, prices, by, sort]);
  const active = rows.find(row => row.key === selected);
  const pages = Math.max(1, Math.ceil(rows.length / 10));
  const current = Math.min(page, pages - 1);
  const detailPages = Math.max(1, Math.ceil((active?.sessions.length ?? 0) / 10));
  const detailPage = Math.min(sessionPage, detailPages - 1);
  const display = (label: string) => by === "model" ? (JSON.parse(label) as string[]).map(value => value || "未知模型").join(" / ") : label;
  return <section aria-label="高消耗模型与项目排行" className="min-w-0 space-y-3 rounded-md border p-3">
    <h3 className="text-sm font-medium">高消耗模型与项目排行</h3>
    <p className="text-xs text-muted-foreground">按当前日期范围和已成功读取的 Codex / Claude 会话统计。Token 不重复计入推理或 1 小时缓存写入；费用只比较当前单价计算出的已知部分，未知费用不表示免费。切换排行不会重新读取文件。</p>
    {!complete && <p role="status" className="text-xs text-amber-700 dark:text-amber-400">分析未完整完成，以下为当前已读取的部分结果。</p>}
    {by === "model" && sessions.some(session => !session.models.length) && <p className="text-xs text-muted-foreground">未记录模型用量的会话无法归入模型排行，可在项目排行中查看。</p>}
    <div className="flex flex-wrap items-center gap-2">
      {(["model", "project"] as const).map(value => <Button key={value} size="sm" variant={by === value ? "default" : "outline"} aria-pressed={by === value} onClick={() => { setBy(value); setPage(0); setSelected(null); }}>{value === "model" ? "模型排行" : "项目排行"}</Button>)}
      <label className="flex min-w-0 items-center gap-2 text-xs">排序<select aria-label="排行排序" className="h-9 min-w-0 rounded border bg-background px-2" value={sort} onChange={event => { setSort(event.target.value as UsageRankingSort); setPage(0); setSelected(null); }}><option value="tokens">Token：最高优先</option><option value="cost">已知费用：最高优先</option></select></label>
    </div>
    {active ? <div aria-label="排行关联会话" className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2"><h4 className="min-w-0 break-all text-sm">{display(active.label)} · {active.sessions.length} 条关联会话</h4><Button size="sm" variant="outline" onClick={() => setSelected(null)}>返回排行</Button></div>
      <p className="text-xs text-muted-foreground">{by === "model" ? "仅显示该模型在当前日期范围内的用量，不包含同一会话的其他模型。" : "显示该项目在当前日期范围内的会话用量。"}</p>
      <p className="text-xs text-muted-foreground">查看正文将打开完整会话，不按排行日期或模型过滤；关闭后返回当前位置。</p>
      {active.sessions.slice(detailPage * 10, (detailPage + 1) * 10).map(session => {
        const summary = summarizeUsage([session], prices);
        const total = summary.tokens.input + summary.tokens.output + summary.tokens.cache_read + summary.tokens.cache_write;
        return <article key={JSON.stringify([session.provider, session.rollout_path])} className="min-w-0 space-y-1 rounded border p-2 text-xs"><h5 className="break-all font-medium">{session.title || session.id}</h5><p className="break-all">{session.provider} · {session.cwd || "未记录项目路径"}</p><p>Token：{summary.overflow || !Number.isSafeInteger(total) || summary.missingUsage ? "无法可靠统计" : total.toLocaleString()} · <Cost summary={summary} /></p><Button size="sm" variant="outline" aria-label={`查看 ${session.title || session.id} 的正文`} onClick={event => onOpenSession(session, event.currentTarget)}>查看正文</Button><details><summary className="cursor-pointer">会话标识与读取提示</summary><p className="break-all">{session.id}</p><p className="break-all">{session.rollout_path}</p>{session.warnings.map((warning, index) => <p className="break-all" key={index}>{warning}</p>)}</details></article>;
      })}
      <nav aria-label="关联会话分页" className="flex flex-wrap items-center gap-2 text-xs"><Button size="sm" variant="outline" disabled={!detailPage} onClick={() => setSessionPage(detailPage - 1)}>上一页会话</Button><span>{detailPage + 1} / {detailPages}</span><Button size="sm" variant="outline" disabled={detailPage + 1 >= detailPages} onClick={() => setSessionPage(detailPage + 1)}>下一页会话</Button></nav>
    </div> : <>
      {!rows.length && <p className="text-sm">当前日期范围没有可排行的用量。</p>}
      {rows.slice(current * 10, (current + 1) * 10).map((row, index) => <article key={row.key} className="min-w-0 space-y-2 rounded border p-2 text-xs"><h4 className="break-all font-medium">{current * 10 + index + 1}. {display(row.label)}</h4><p>Token：{row.totalTokens === null ? "无法可靠统计" : row.totalTokens.toLocaleString()} · <Cost summary={row.summary} /></p><p>{row.sessions.length} 条会话{row.summary.unknownModels > 0 ? ` · ${row.summary.unknownModels} 项用量缺少单价` : ""}{row.summary.missingUsage > 0 ? ` · ${row.summary.missingUsage} 条会话未记录用量` : ""}{row.summary.warnedSessions > 0 ? ` · ${row.summary.warnedSessions} 条会话有读取提示` : ""}</p><Button size="sm" variant="outline" aria-label={`查看 ${display(row.label)} 的关联会话`} onClick={() => { setSelected(row.key); setSessionPage(0); }}>查看关联会话</Button></article>)}
      <nav aria-label="排行分页" className="flex flex-wrap items-center gap-2 text-xs"><Button size="sm" variant="outline" disabled={!current} onClick={() => setPage(current - 1)}>上一页排行</Button><span>{current + 1} / {pages} · {rows.length} 项</span><Button size="sm" variant="outline" disabled={current + 1 >= pages} onClick={() => setPage(current + 1)}>下一页排行</Button></nav>
    </>}
  </section>;
}
