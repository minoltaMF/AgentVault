import { useMemo } from "react";
import { Button } from "@/components/ui/button";
import { compareUsagePeriods } from "@/lib/usage-period-comparison";
import type { PriceBook, UsageDateRange, UsageSession } from "@/lib/session-usage";

const count = (value: number) => value.toLocaleString();
const money = (value: number) => `$${value.toLocaleString("en-US", { maximumFractionDigits: 6 })}`;

function Change({ delta, percent, reason, cost = false }: { delta: number | null; percent: number | null; reason: string | null; cost?: boolean }) {
  if (delta === null) return <span className="text-muted-foreground">暂不比较</span>;
  const amount = cost ? money(Math.abs(delta)) : count(Math.abs(delta));
  return <span>{cost && reason ? "已知差额 " : ""}{delta > 0 ? "+" : delta < 0 ? "−" : ""}{amount}{percent !== null && <span className="ml-2 text-muted-foreground">（{percent > 0 ? "+" : ""}{percent.toLocaleString(undefined, { maximumFractionDigits: 2 })}%）</span>}</span>;
}

export function UsagePeriodComparison({ sessions, prices, analysisComplete, onInspect, selection, onSelectionChange }: {
  sessions: UsageSession[]; prices: PriceBook; analysisComplete: boolean; onInspect: (range: UsageDateRange) => void;
  selection: { days: 7 | 30; now: Date }; onSelectionChange: (selection: { days: 7 | 30; now: Date }) => void;
}) {
  const comparison = useMemo(() => compareUsagePeriods(sessions, prices, selection.days, selection.now, analysisComplete), [sessions, prices, selection, analysisComplete]);
  const { current, previous, tokensChange, costChange } = comparison;
  const notices = [...new Set([tokensChange.reason, costChange.reason].filter((reason): reason is string => !!reason))];
  return <section aria-label="用量周期对比" className="space-y-3 rounded-md border p-3">
    <div className="flex flex-wrap items-center gap-2">
      <h3 className="mr-auto text-sm font-medium">周期对比</h3>
      {([7, 30] as const).map(days => <Button key={days} size="sm" variant={selection.days === days ? "default" : "outline"} aria-pressed={selection.days === days} onClick={() => onSelectionChange({ days, now: new Date() })}>对比近 {days} 天</Button>)}
    </div>
    <p className="text-xs text-muted-foreground">基于当前工作台筛选的 Codex / Claude 会话，与下方日期筛选独立。两个周期相邻且不重叠，均按 UTC 日统计；本期包含截止日期当天已记录用量，当天可能尚未结束，不外推全天。</p>
    <div className="grid gap-3 sm:grid-cols-2">
      {([{ label: "本期", period: current }, { label: "上期", period: previous }] as const).map(({ label, period }) => <div key={label} className="space-y-2 rounded-md bg-muted/40 p-3">
        <p className="text-sm font-medium">{label} · {selection.days} 天</p>
        <p className="text-xs tabular-nums">{period.range.from} — {period.range.to}</p>
        <p className="text-xs text-muted-foreground">{period.sessionCount} 条会话 · {period.recordedDays} 天有用量记录</p>
        <Button size="sm" variant="outline" onClick={() => onInspect(period.range)}>查看{label}明细</Button>
      </div>)}
    </div>
    <div className="overflow-x-auto"><table className="w-full min-w-[480px] text-left text-xs [&_th]:px-2 [&_td]:px-2"><caption className="sr-only">本期与上期用量比较</caption>
      <thead><tr className="border-b"><th className="py-2">指标</th><th>本期</th><th>上期</th><th>本期 − 上期</th></tr></thead>
      <tbody>
        <tr className="border-b"><th className="py-3 font-medium">已记录 Token</th>{[current, previous].map((period, index) => <td key={index}>{period.overflow ? "超出安全范围" : period.hasUsage ? count(period.tokens) : "未记录用量"}</td>)}<td><Change {...tokensChange} /></td></tr>
        <tr><th className="py-3 font-medium">已知费用（USD）</th>{[current, previous].map((period, index) => <td key={index}>{period.overflow ? "无法计算" : !period.hasUsage ? "未记录用量" : <>{money(period.known)}{period.unknownModels > 0 && <span className="block text-muted-foreground">{period.unknownModels} 项缺少单价</span>}</>}</td>)}<td><Change {...costChange} cost /></td></tr>
      </tbody>
    </table></div>
    {notices.length > 0 && <div role="status" className="space-y-1 text-xs text-amber-700 dark:text-amber-400">{notices.map(notice => <p key={notice}>{notice}</p>)}</div>}
    <p className="text-xs text-muted-foreground">缺日不代表零消耗；差额只比较已记录用量。费用使用两期相同的当前录入单价，价格修改会同步重算，不等于历史账单。切换周期不重新读取文件。</p>
  </section>;
}
