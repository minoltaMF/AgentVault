import { useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import type { UsageCacheStatus } from "@/lib/session-usage";
import { Button } from "@/components/ui/button";

const labels: Record<UsageCacheStatus["state"], string> = { ready: "可用（最近一次校验）", missing: "尚未生成", degraded: "已降级为内存缓存", disabled: "未配置持久缓存", unchecked: "等待校验" };
function bytes(value: number | null) {
  if (value === null) return "未知";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(2)} MiB`;
}

export function UsageCachePanel({ busy, refreshKey, onRebuilt, analysisWarning }: { analysisWarning: string | null; busy: boolean; refreshKey: string; onRebuilt: () => void }) {
  const [cache, setCache] = useState<UsageCacheStatus | null>(null);
  const [error, setError] = useState("");
  const [rebuilding, setRebuilding] = useState(false);
  const [revision, setRevision] = useState(0);
  const [notice, setNotice] = useState("");
  const mounted = useRef(true);
  const request = useRef(0);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; request.current++; }; }, []);
  useEffect(() => {
    const id = ++request.current;
    void api.sessionUsageCacheStatus().then(value => {
      if (mounted.current && id === request.current) { setCache(value); setError(""); }
    }).catch(failure => {
      if (mounted.current && id === request.current) setError(`无法读取缓存状态：${String(failure)}`);
    });
  }, [refreshKey, revision]);
  async function rebuild() {
    if (busy || rebuilding) return;
    setRebuilding(true); setError(""); setNotice(""); request.current++;
    try {
      const next = await api.rebuildSessionUsageCache();
      if (!mounted.current) return;
      setCache(next);
      setNotice(next.backup_path ? "旧缓存已备份，正在重新分析当前筛选范围。" : "缓存已重置，正在重新分析当前筛选范围。");
      onRebuilt();
    } catch (failure) {
      if (mounted.current) {
        setError(`重建未完成：${String(failure)}`);
        // A failed replacement can still have created a verified recovery copy.
        const next = await api.sessionUsageCacheStatus().catch(() => null);
        if (mounted.current && next) setCache(next);
      }
    } finally { if (mounted.current) setRebuilding(false); }
  }
  return <section aria-label="用量缓存管理" className="space-y-2 rounded-md border p-3 text-xs">
    <div className="flex flex-wrap items-center justify-between gap-2">
      <h3 className="text-sm font-medium">用量缓存</h3>
      <Button size="sm" variant="outline" disabled={rebuilding} onClick={() => setRevision(value => value + 1)}>刷新缓存状态</Button>
    </div>
    <p role="status">{cache ? `${labels[cache.state] ?? "未知状态"} · 磁盘 ${bytes(cache.disk_bytes)} · 内存 ${cache.memory_entries.toLocaleString()} 项（约 ${bytes(cache.memory_bytes)}）` : "正在读取缓存状态…"}</p>
    {(cache?.warning || analysisWarning) && <p className="break-all text-amber-700 dark:text-amber-400">{cache?.warning || analysisWarning}</p>}
    <p className="text-muted-foreground">安全重建会先备份现有缓存，再清空全部用量缓存并重新分析当前筛选范围。原生会话和模型单价保持不变，其他范围下次打开时重新生成。</p>
    <Button size="sm" variant="outline" disabled={busy || rebuilding || !cache || cache.state === "disabled"} onClick={() => void rebuild()}>{rebuilding ? "正在安全重建…" : "安全重建缓存"}</Button>
    {busy && <p className="text-muted-foreground">分析完成或取消结束后可以重建。</p>}
    {notice && <p role="status">{busy ? notice : "缓存已重建；分析结果见上方状态。"}</p>}
    {error && <p role="alert" className="break-all text-destructive">{error}</p>}
    {(cache?.path || cache?.backup_path) && <details><summary className="cursor-pointer">缓存与备份位置</summary>{cache.path && <p className="mt-2 break-all">缓存：{cache.path}</p>}{cache.backup_path && <p className="mt-2 break-all">最近备份：{cache.backup_path}</p>}</details>}
  </section>;
}
