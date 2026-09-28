import { useLayoutEffect, useRef, useState } from "react";
import type { useWorkbenchSource } from "@/hooks/useWorkbenchSource";
import type { WorkbenchView } from "@/lib/workbenchCache";
import { scanProgressText } from "@/lib/allSessions";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { SettingsSheet } from "@/components/SettingsSheet";

type Source = { name: string; root: string; data: ReturnType<typeof useWorkbenchSource> };
export function WorkbenchSources({ sources, view }: { sources: Source[]; view: WorkbenchView }) {
  const [unusedOpen, setUnusedOpen] = useState(view.sourcesExpanded ?? false);
  const [expanded, setExpanded] = useState<string[]>(view.sourceDetails ?? []);
  const pendingFocus = useRef<string | null>(null);
  useLayoutEffect(() => {
    if (pendingFocus.current) {
      document.getElementById(`source-heading-${pendingFocus.current}`)?.focus();
      pendingFocus.current = null;
    }
  });
  const used = sources.filter(s => s.data.used || s.data.state !== "idle");
  const unused = sources.filter(s => !s.data.used && s.data.state === "idle").sort((a,b) => Number(!!b.root) - Number(!!a.root));
  const configured = sources.filter(s => !!s.root).length;
  const toggle = (name: string) => {
    const next = expanded.includes(name) ? expanded.filter(n => n !== name) : [...expanded, name];
    view.sourceDetails = next; setExpanded(next);
  };
  return <section aria-label="来源管理" className="min-w-0 rounded-lg border">
    <div className="flex flex-wrap items-center justify-between gap-2 p-3">
      <div><h2 className="text-sm font-medium">会话来源 <span className="ml-2 text-xs font-normal text-muted-foreground">已配置 {configured} · 已使用 {used.length}</span></h2>
      <p className="mt-1 text-xs text-muted-foreground">已配置表示已设置目录；读取后确认是否有会话。</p></div>
      <SettingsSheet trigger={<Button size="sm" variant="outline">配置来源</Button>} />
    </div>
    {used.length > 0 && <div className="divide-y border-t">{used.map(source => <SourceRow key={source.name} source={source} expanded={expanded.includes(source.name)} toggle={() => toggle(source.name)} />)}</div>}
    {!used.length && <p className="px-3 pb-3 text-sm text-muted-foreground">展开下方来源选择读取，或使用顶部刷新读取全部已配置来源。</p>}
    {unused.length > 0 && <div className="border-t">
      <button type="button" aria-expanded={unusedOpen} aria-controls="unused-workbench-sources" className="flex w-full items-center justify-between gap-2 rounded-b-lg p-3 text-left text-sm hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" onClick={() => { view.sourcesExpanded = !unusedOpen; setUnusedOpen(!unusedOpen); }}>
        <span>未使用来源（{unused.length}）<span className="ml-2 text-xs text-muted-foreground">{unused.filter(s => !!s.root).length} 个已配置</span></span><span>{unusedOpen ? "收起" : "展开"}</span>
      </button>
      <div id="unused-workbench-sources" hidden={!unusedOpen} className="divide-y border-t">{unused.map(source => <SourceRow onRead={() => { pendingFocus.current = source.name.replace(/[^a-zA-Z0-9]/g, "-"); view.sourcesExpanded = false; setUnusedOpen(false); }} key={source.name} source={source} expanded={expanded.includes(source.name)} toggle={() => toggle(source.name)} />)}</div>
    </div>}
  </section>;
}
function SourceRow({ source: { name, root, data }, expanded, toggle, onRead }: { source: Source; expanded: boolean; toggle: () => void; onRead?: () => void }) {
  const attention = !!data.error || !!data.progress?.failed_files || data.state === "interrupted";
  const status = !root ? "未配置" : data.state === "interrupted" ? "状态待确认" : data.state === "cancelled" ? "已停止" : data.state === "loading" ? data.cancelling ? "正在停止" : "正在读取" : data.state === "error" ? "读取失败" : data.state === "ready" ? `${data.progress?.failed_files ? "部分完成 · " : ""}已读取 ${data.sessions.length} 条` : "已配置 · 尚未读取";
  return <section aria-label={`${name} 来源`} className={`min-w-0 p-3 ${root ? "" : "bg-muted/20"}`}>
    <div className="flex flex-wrap items-center gap-2">
      <h3 id={`source-heading-${name.replace(/[^a-zA-Z0-9]/g, "-")}`} tabIndex={-1} className="text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{name}</h3><Badge variant={attention ? "destructive" : "outline"}>{status}</Badge>
      <div className="ml-auto flex flex-wrap items-center gap-2">
        <Button size="sm" variant="ghost" aria-expanded={expanded || attention} aria-controls={`source-detail-${name.replace(/[^a-zA-Z0-9]/g, "-")}`} onClick={toggle} disabled={attention}>{attention ? "详情已展开" : expanded ? "收起详情" : "查看详情"}<span className="sr-only"> {name}</span></Button>
            <div className="flex flex-wrap gap-2">
              <Button size="sm" variant="outline" disabled={!root || data.state === "loading" || data.state === "interrupted"} onClick={() => { onRead?.(); void data.refresh(); }}>{data.state === "loading" ? "正在读取…" : data.state === "error" ? `重试 ${name}` : data.checkedAt ? `刷新 ${name}` : `读取 ${name}`}</Button>
              {(data.state === "loading" || data.state === "interrupted") && <Button size="sm" variant="outline" disabled={data.cancelling} onClick={() => { void data.cancel(); }}>{data.cancelling ? "正在停止…" : `停止 ${name} 扫描`}</Button>}
              {data.state === "interrupted" && <Button size="sm" variant="outline" onClick={data.retry}>重试查询 {name}</Button>}
            </div>

      </div>
    </div>
    {data.state === "loading" && data.progress && !expanded && !attention && <p role="status" className="mt-2 text-xs">{scanProgressText(data.progress)}</p>}
    <div id={`source-detail-${name.replace(/[^a-zA-Z0-9]/g, "-")}`} hidden={!expanded && !attention} className="mt-2 space-y-2 border-t pt-2">
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

    </div>
  </section>;
}
