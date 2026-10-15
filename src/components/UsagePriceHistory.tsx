import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { rateFields, type PriceBook } from "@/lib/session-usage";
import type { PriceHistoryEntry } from "@/lib/price-history";

const reasons = { edit: "批量编辑", import: "配置导入", restore: "历史恢复" };
export function UsagePriceHistory({ history, prices, disabled, error, onRestore }: { history: PriceHistoryEntry[]; prices: PriceBook; disabled: boolean; error: string; onRestore: (id: string) => Promise<void> }) {
  const [selected, setSelected] = useState<string | null>(null);
  const [failure, setFailure] = useState("");
  const [working, setWorking] = useState(false);
  const entry = history.find(item => item.id === selected);
  const changes = useMemo(() => {
    if (!entry) return [];
    return [...new Set([...Object.keys(prices), ...Object.keys(entry.before)])].flatMap(key => rateFields.flatMap(field => {
      const before = prices[key]?.[field], after = entry.before[key]?.[field];
      return before === after ? [] : [{ key, field, before, after }];
    }));
  }, [entry, prices]);
  async function restore() {
    if (!entry || disabled || working) return;
    setWorking(true); setFailure("");
    try { await onRestore(entry.id); setSelected(null); }
    catch (cause) { setFailure(`恢复失败：${String(cause)}。当前配置未被此恢复覆盖。`); }
    finally { setWorking(false); }
  }
  return <details className="space-y-2 rounded border p-2" aria-label="单价变更历史">
    <summary className="cursor-pointer text-sm">变更历史与恢复（{history.length}）</summary>
    <p className="text-xs text-muted-foreground">仅记录启用本功能后的成功变更，保存在本机，最多保留最近 20 条且总计不超过 2 MiB，超限时淘汰最旧记录。恢复会替换整份单价配置，并记录恢复前配置以便撤回；不修改会话。请先保存或放弃草稿及导入预览。</p>
    {error && <p role="alert" className="break-all text-xs text-destructive">{error}</p>}
    {!history.length && !error && <p className="text-xs">暂无变更历史。首次修改前的配置会自动保留。</p>}
    {history.map(item => <div key={item.id} className="flex flex-wrap items-center justify-between gap-2 border-b py-2 text-xs"><span>{new Date(item.time).toLocaleString()} · {reasons[item.reason]}</span><Button size="sm" variant="outline" disabled={disabled || working} onClick={() => { setSelected(item.id); setFailure(""); }}>恢复到此操作之前</Button></div>)}
    {entry && <div aria-label="单价恢复预览" className="space-y-2 rounded border p-2 text-xs"><p>将恢复至 {new Date(entry.time).toLocaleString()} 的{reasons[entry.reason]}之前，涉及 {changes.length} 个单价字段变化。此操作将替换当前全部模型配置，包括移除当时不存在的模型和单价。</p><div className="max-h-48 space-y-1 overflow-y-auto">{changes.map(change => <p key={`${change.key}:${change.field}`} className="break-all">{change.key} / {change.field}：{change.before ?? "未知"} → {change.after ?? "未知"}</p>)}</div><div className="flex flex-wrap gap-2"><Button size="sm" disabled={disabled || working} onClick={() => void restore()}>{working ? "正在恢复…" : "确认恢复"}</Button><Button size="sm" variant="outline" disabled={working} onClick={() => setSelected(null)}>取消恢复</Button></div></div>}
    {failure && <p role="alert" className="break-all text-xs text-destructive">{failure}</p>}
  </details>;
}
