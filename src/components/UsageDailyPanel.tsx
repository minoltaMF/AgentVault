import { useMemo, useState } from "react";
import type { EChartsCoreOption } from "echarts/core";
import { EChart } from "@/components/EChart";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useTheme } from "@/stores/theme";
import { dailyUsageTrend, recentUsageRange, undatedUsageSessions, validateUsageDateRange, type PriceBook, type UsageDateRange, type UsageSession } from "@/lib/session-usage";

const money = (value: number) => `$${value.toLocaleString("en-US", { maximumFractionDigits: 6 })}`;

export function UsageDailyPanel({ sessions, prices, range, onChange }: {
  sessions: UsageSession[]; prices: PriceBook; range: UsageDateRange; onChange: (range: UsageDateRange) => void;
}) {
  const [metric, setMetric] = useState<"tokens" | "cost">("tokens");
  const [sort, setSort] = useState<"date" | "tokens" | "cost">("date");
  const [page, setPage] = useState(0);
  const theme = useTheme(state => state.resolved);
  const error = validateUsageDateRange(range);
  const isRecent = (days: number) => { const recent = recentUsageRange(days); return range.from === recent.from && range.to === recent.to; };
  const days = useMemo(() => dailyUsageTrend(sessions, prices, range), [sessions, prices, range]);
  const undated = useMemo(() => undatedUsageSessions(sessions), [sessions]);
  const rows = useMemo(() => [...days].sort((a, b) => {
    if (sort === "date") return b.date.localeCompare(a.date);
    if (a.overflow !== b.overflow) return a.overflow ? 1 : -1;
    return (sort === "tokens" ? b.tokens - a.tokens : b.known - a.known) || b.date.localeCompare(a.date);
  }), [days, sort]);
  const peak = useMemo(() => days.filter(day => !day.overflow).reduce<typeof days[number] | undefined>((best, day) => !best || day.tokens > best.tokens ? day : best, undefined), [days]);
  const option = useMemo<EChartsCoreOption>(() => {
    const visible = days.slice(-90);
    const color = theme === "dark" ? "#a1a1aa" : "#52525b";
    return {
      animation: false,
      grid: { top: 30, bottom: 36, left: 12, right: 12, containLabel: true },
      tooltip: { trigger: "axis", renderMode: "richText", confine: true },
      xAxis: { type: "category", data: visible.map(day => day.date), axisLabel: { color, hideOverlap: true, formatter: (value: string) => value.slice(5) } },
      yAxis: { type: "value", name: metric === "tokens" ? "Token" : "USD（已知部分）", nameTextStyle: { color }, axisLabel: { color }, splitLine: { lineStyle: { color: theme === "dark" ? "#3f3f46" : "#e4e4e7" } } },
      series: [{ type: "bar", name: metric === "tokens" ? "Token" : "估算费用（已知部分 USD）", itemStyle: { color: metric === "tokens" ? "#3b82f6" : "#14b8a6" }, data: visible.map(day => day.overflow ? null : metric === "tokens" ? day.tokens : day.known === 0 && day.incomplete ? null : day.known) }],
    };
  }, [days, metric, theme]);
  const pages = Math.max(1, Math.ceil(rows.length / 10));
  const current = Math.min(page, pages - 1);
  function change(next: UsageDateRange) { setPage(0); onChange(next); }
  return <section className="space-y-3 rounded-md border p-3" aria-label="日期筛选与按日趋势">
    <div className="flex flex-wrap items-center gap-2">
      <h3 className="mr-auto text-sm font-medium">日期筛选与按日趋势</h3>
      <Button size="sm" variant={!range.from && !range.to ? "default" : "outline"} onClick={() => change({ from: "", to: "" })}>全部日期</Button>
      <Button size="sm" variant={isRecent(7) ? "default" : "outline"} aria-pressed={isRecent(7)} onClick={() => change(recentUsageRange(7))}>近 7 天</Button>
      <Button size="sm" variant={isRecent(30) ? "default" : "outline"} aria-pressed={isRecent(30)} onClick={() => change(recentUsageRange(30))}>近 30 天</Button>
    </div>
    <div className="grid gap-3 sm:grid-cols-2">
      <label className="space-y-1 text-xs">开始日期（UTC）<Input type="date" aria-label="用量开始日期" value={range.from} onChange={event => change({ ...range, from: event.target.value })} /></label>
      <label className="space-y-1 text-xs">结束日期（UTC，含当天）<Input type="date" aria-label="用量结束日期" value={range.to} onChange={event => change({ ...range, to: event.target.value })} /></label>
    </div>
    <p className="text-xs text-muted-foreground">按原生用量记录的 UTC 日期统计，近 7 / 30 天包含今天；下方汇总和明细同步筛选。Token 为输入、输出、缓存读取与写入之和，不重复计算推理。</p>
    {undated.length > 0 && <p className="text-xs text-amber-700 dark:text-amber-400">{undated.length} 条会话含日期未知的用量：仅计入「全部日期」汇总，不进入日期筛选与趋势。缺失日期不会用文件修改时间代替。</p>}
    {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : !days.length ? <p className="text-sm text-muted-foreground">此范围没有可按日统计的用量记录。</p> : <>
      <div className="flex flex-wrap items-center gap-2"><span className="mr-auto text-xs">{days.length} 个有用量记录的日期{peak ? ` · Token 最高：${peak.date}（${peak.tokens.toLocaleString()}）` : ""}</span>{(["tokens", "cost"] as const).map(value => <Button key={value} size="sm" variant={metric === value ? "default" : "outline"} aria-pressed={metric === value} onClick={() => setMetric(value)}>{value === "tokens" ? "Token 趋势" : "费用趋势"}</Button>)}</div>
      <div role="img" aria-label={`${metric === "tokens" ? "Token" : "已知费用"}按日趋势，数据见下方每日明细`} className="h-52"><EChart option={option} /></div>
      <p className="text-xs text-muted-foreground">仅展示有记录的日期，缺日不代表零消耗。{days.length > 90 ? "图表展示最近 90 个有记录日期，表格保留全部日期。" : ""}费用采用当前录入单价；未知或不完整费用不会视为完整账单，费用排序比较已知部分。</p>
      <label className="flex items-center gap-2 text-xs">每日明细排序<select className="h-8 rounded-md border bg-background px-2" aria-label="每日明细排序" value={sort} onChange={event => { setSort(event.target.value as typeof sort); setPage(0); }}><option value="date">日期：最新优先</option><option value="tokens">Token：最高优先</option><option value="cost">已知费用：最高优先</option></select></label>
      <div className="overflow-x-auto"><table className="w-full min-w-[380px] text-left text-xs [&_th]:px-2 [&_td]:px-2"><caption className="sr-only">每日用量与估算费用</caption><thead><tr className="border-b"><th className="py-2">日期（UTC）</th><th>会话</th><th>Token</th><th>估算费用（USD）</th></tr></thead><tbody>{rows.slice(current * 10, (current + 1) * 10).map(day => <tr key={day.date} className="border-b last:border-0"><td className="whitespace-nowrap py-2"><button className="text-primary underline underline-offset-2" aria-label={`查看 ${day.date} 用量`} onClick={() => change({ from: day.date, to: day.date })}>{day.date}</button></td><td>{day.sessions}</td><td>{day.overflow ? "超出安全范围" : day.tokens.toLocaleString()}</td><td>{day.overflow ? "无法计算" : day.known === 0 && day.incomplete ? "未知（已知部分为 0）" : `${money(day.known)}${day.incomplete ? "（已知部分）" : ""}`}</td></tr>)}</tbody></table></div>
      {pages > 1 && <nav aria-label="每日用量分页" className="flex items-center justify-center gap-3"><Button size="sm" variant="outline" disabled={!current} onClick={() => setPage(current - 1)}>前 10 天</Button><span className="text-xs">{current + 1} / {pages}</span><Button size="sm" variant="outline" disabled={current + 1 >= pages} onClick={() => setPage(current + 1)}>后 10 天</Button></nav>}
      <p className="text-xs text-muted-foreground">点击表格日期查看当天的会话、模型与项目用量；选择「全部日期」返回全量。</p>
    </>}
  </section>;
}
