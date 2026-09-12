import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { AlertTriangle, Archive, CheckCircle2, Copy, FolderOpen, Loader2, RotateCcw, ShieldCheck } from "lucide-react";
import { api, type DeleteSnapshotDetail, type DeleteSnapshotSummary } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { joinPath } from "@/lib/cwd";
import { deleteSnapshotStatus, recoveryFolderError, readSnapshotActivities, saveSnapshotActivity, type SnapshotActivities, type SnapshotActivity } from "@/lib/deleteSnapshots";
import { pickDirectoryPath } from "@/lib/dialog";
import { humanBytes } from "@/lib/format";
import { useSettings } from "@/stores/settings";
import { BackupNavigation } from "@/components/BackupNavigation";
import { TopBar } from "@/components/TopBar";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { EmptyState } from "@/components/EmptyState";

type CheckResult = { ok: boolean; time: string; error?: string };
type RestoreResult = { ok: boolean; output: string; time: string; error?: string };
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const dateText = (value: string | null) => value && Number.isFinite(Date.parse(value)) ? new Date(value).toLocaleString() : "时间未知";

export default function DeleteSnapshotsRoute() {
  const backupDir = useSettings((state) => state.settings?.backup_dir ?? "");
  // A changed backup root mounts a fresh scope; late responses cannot populate the new source.
  return <SnapshotBrowser key={backupDir} backupDir={backupDir} />;
}

function SnapshotBrowser({ backupDir }: { backupDir: string }) {
  const [params, setParams] = useSearchParams();
  const selectedName = params.get("snapshot");
  const [items, setItems] = useState<DeleteSnapshotSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [detail, setDetail] = useState<DeleteSnapshotDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState("");
  const [check, setCheck] = useState<CheckResult | null>(null);
  const [restore, setRestore] = useState<RestoreResult | null>(null);
  const [busy, setBusy] = useState<"verify" | "restore" | null>(null);
  const [parent, setParent] = useState("");
  const [folder, setFolder] = useState("recovered-codex");
  const [utilityMessage, setUtilityMessage] = useState("");
  const [history, setHistory] = useState<SnapshotActivities>({});
  const mounted = useRef(true);
  const listRequest = useRef(0);
  const detailRequest = useRef(0);
  const operation = useRef(false);
  const selected = items.find((item) => item.name === selectedName);
  const heading = useRef<HTMLHeadingElement>(null);

  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const refresh = useCallback(async () => {
    if (operation.current) return;
    const request = ++listRequest.current;
    setLoading(true); setError(""); setCheck(null);
    try {
      const next = backupDir ? await api.listDeleteSnapshots(backupDir) : [];
      if (mounted.current && request === listRequest.current) setItems(next);
    } catch (error) {
      if (mounted.current && request === listRequest.current) setError(message(error));
    } finally {
      if (mounted.current && request === listRequest.current) setLoading(false);
    }
  }, [backupDir]);
  useEffect(() => { void refresh(); }, [refresh]);

  useEffect(() => {
    const request = ++detailRequest.current;
    setDetail(null); setDetailError(""); setCheck(null); setRestore(null); setUtilityMessage("");
    if (!selected) { setDetailLoading(false); return; }
    try { setHistory(readSnapshotActivities(window.localStorage, selected.snapshot_path)); } catch { setHistory({}); }
    setDetailLoading(true);
    void api.inspectDeleteSnapshot(backupDir, selected.snapshot_path).then((value) => {
      if (mounted.current && request === detailRequest.current) setDetail(value);
    }).catch((error) => {
      if (mounted.current && request === detailRequest.current) setDetailError(message(error));
    }).finally(() => {
      if (mounted.current && request === detailRequest.current) setDetailLoading(false);
    });
  }, [backupDir, selected]);

  const run = async (kind: "verify" | "restore") => {
    if (!detail || operation.current) return;
    if (kind === "restore" && (!parent.trim() || recoveryFolderError(folder))) return;
    operation.current = true; setBusy(kind); setUtilityMessage("");
    const request = detailRequest.current;
    const output = joinPath(parent.trim(), folder);
    const remember = (item: SnapshotActivity) => {
      try {
        const next = saveSnapshotActivity(window.localStorage, detail.snapshot_path, kind, item);
        if (mounted.current && request === detailRequest.current) setHistory(next);
      } catch {
        if (mounted.current && request === detailRequest.current) setUtilityMessage("浏览器无法保存操作记录；请保留本次结果和目录路径。");
      }
    };
    if (kind === "verify") setCheck(null); else setRestore(null);
    try {
      const report = kind === "verify"
        ? await api.verifyDeleteSnapshot(backupDir, detail.snapshot_path)
        : await api.restoreDeleteSnapshot(backupDir, detail.snapshot_path, output);
      if (!report.verified) throw new Error("校验未通过");
      remember({ ok: true, time: new Date().toISOString(), ...(kind === "restore" ? { output } : {}) });
      if (mounted.current && request === detailRequest.current) {
        const time = new Date().toISOString();
        setCheck({ ok: true, time });
        if (kind === "restore") setRestore({ ok: true, output, time });
      }
    } catch (error) {
      remember({ ok: false, time: new Date().toISOString(), error: message(error), ...(kind === "restore" ? { output } : {}) });
      if (mounted.current && request === detailRequest.current) {
        const time = new Date().toISOString();
        if (kind === "verify") setCheck({ ok: false, time, error: message(error) });
        else setRestore({ ok: false, output, time, error: message(error) });
      }
    } finally {
      operation.current = false;
      if (mounted.current) setBusy(null);
    }
  };
  const utility = async (action: () => Promise<void>, success: string) => {
    try { await action(); if (mounted.current) setUtilityMessage(success); }
    catch (error) { if (mounted.current) setUtilityMessage(message(error)); }
  };
  const output = parent.trim() && !recoveryFolderError(folder) ? joinPath(parent.trim(), folder) : "";

  return <>
    <TopBar title="Codex 删除前快照" stats={`${items.length} 份`} refreshing={loading || !!busy} onRefresh={() => { void refresh(); }} />
    <BackupNavigation snapshots />
    <ScrollArea className="min-h-0 flex-1">
      <div className="space-y-5 p-6">
        <div className="space-y-2 text-sm text-muted-foreground">
          <p>删除前自动保存的恢复材料。可查看、重新校验，或恢复到新的隔离目录。</p>
          <p className="break-all text-xs">备份目录：{backupDir || "尚未配置"}</p>
          <p className="text-xs">列表读取元数据；“尚未校验”不代表损坏。恢复会再次完整校验，不覆盖原会话。</p>
        </div>
        {error && <div role="alert" className="space-y-2 rounded-md border border-destructive/40 p-3 text-sm">
          <p className="break-all">读取快照列表失败：{error}</p>
          <Button variant="outline" size="sm" disabled={loading || !!busy} onClick={() => { void refresh(); }}>重试</Button>
        </div>}
        {loading && <p role="status" className="flex items-center gap-2 text-sm"><Loader2 className="h-4 w-4 animate-spin" />正在读取快照列表…</p>}
        {!loading && !error && items.length === 0 && <EmptyState icon={<Archive className="h-9 w-9" />} title="还没有删除前快照" description="Codex 会话删除前会自动创建。已有普通备份可从上方分类查看。" />}
        {!loading && selectedName && !selected && !error && <p role="alert" className="rounded-md border p-3 text-sm">未找到指定快照。它可能已移动，或当前备份目录已更改。</p>}
        {items.length > 0 && <div className="grid min-w-0 gap-5 xl:grid-cols-[minmax(260px,0.8fr)_minmax(0,1.2fr)]">
          <section aria-label="删除快照列表" className="min-w-0 space-y-3">
            {items.map((item) => <article key={item.snapshot_path} className={`min-w-0 rounded-lg border p-4 ${item.name === selectedName ? "border-primary bg-muted/30" : ""}`}>
              <div className="flex flex-wrap items-center justify-between gap-2"><span className="text-sm font-medium">{dateText(item.created_at)}</span><Badge variant="outline">{deleteSnapshotStatus(item.status)}</Badge></div>
              <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{item.name}</p>
              <p className="mt-2 text-xs">{item.session_ids.length} 条会话 · {item.files} 个文件 · 声明大小 {humanBytes(item.total_bytes)}</p>
              {item.source_root && <p className="mt-1 truncate text-xs text-muted-foreground" title={item.source_root}>来源：{item.source_root}</p>}
              {item.error && <p className="mt-2 break-all text-xs text-destructive">{item.error}</p>}
              <Button className="mt-3" variant="outline" size="sm" disabled={!!busy} onClick={() => { setParams({ snapshot: item.name }); window.setTimeout(() => heading.current?.focus(), 0); }}>查看快照</Button>
            </article>)}
          </section>
          <section aria-label="快照详情" className="min-w-0 space-y-4 rounded-lg border p-4">
            <h2 ref={heading} tabIndex={-1} className="text-base font-semibold outline-none">快照详情</h2>
            {!selected && <p className="text-sm text-muted-foreground">选择一份快照，查看内容或开始恢复。</p>}
            {detailLoading && <p role="status" className="text-sm">正在读取详情…</p>}
            {detailError && <p role="alert" className="break-all text-sm text-destructive">读取详情失败：{detailError}</p>}
            {detail && <>
              <div className="space-y-2 text-sm">
                <p className="break-all font-mono text-xs">{detail.snapshot_path}</p>
                <div className="flex flex-wrap gap-2">
                  <Button size="sm" variant="outline" onClick={() => { void utility(() => copyText(detail.snapshot_path), "快照路径已复制"); }}><Copy className="mr-1 h-4 w-4" />复制路径</Button>
                  <Button size="sm" variant="outline" onClick={() => { void utility(() => api.revealCwd(detail.snapshot_path), "已请求在服务所在电脑打开快照目录"); }}><FolderOpen className="mr-1 h-4 w-4" />打开快照目录</Button>
                </div>
                {detail.error && <p role="alert" className="break-all text-destructive">{detail.error}</p>}
                <p className="break-all text-xs text-muted-foreground">原来源：{detail.source_root ?? "无法读取"}</p>
                <details><summary className="cursor-pointer text-xs">涉及 {detail.session_ids.length} 条会话</summary><ul className="mt-2 max-h-32 overflow-auto font-mono text-xs">{detail.session_ids.map((id) => <li className="break-all" key={id}>{id}</li>)}</ul></details>
              </div>
              <div className="space-y-2 border-t pt-4">
                <Button size="sm" disabled={!!busy || detail.status !== "unverified"} onClick={() => { void run("verify"); }}><ShieldCheck className="mr-1 h-4 w-4" />重新校验</Button>
                {busy && <p role="status" className="flex items-center gap-2 text-sm"><Loader2 className="h-4 w-4 animate-spin" />{busy === "verify" ? "正在校验所有文件和数据库…" : "正在校验并恢复，请等待完成结果…"}</p>}
                {!check && !busy && <p className="text-xs text-muted-foreground">本次打开尚未执行完整校验。</p>}
                {check && <div role={check.ok ? "status" : "alert"} className="space-y-1 text-sm">
                  <p className="flex items-center gap-2">{check.ok ? <CheckCircle2 className="h-4 w-4 text-emerald-600" /> : <AlertTriangle className="h-4 w-4 text-destructive" />}{check.ok ? "校验通过" : "校验失败"} · {dateText(check.time)}</p>
                  {check.error && <p className="break-all text-destructive">{check.error}</p>}
                  <p className="text-xs text-muted-foreground">结果对应此次检查；恢复前仍会重新校验。</p>
                </div>}
              </div>
              <details open className="border-t pt-4"><summary className="cursor-pointer text-sm font-medium">文件清单（{detail.members.length} 项）</summary>
                <div className="mt-2 max-h-64 overflow-auto"><table className="w-full text-left text-xs"><thead><tr><th className="py-2">文件</th><th className="px-2">类型</th><th className="text-right">大小 / 状态</th></tr></thead>
                  <tbody>{detail.members.map((member) => <tr key={member.path} className="border-t"><td className="break-all py-2 font-mono">{member.path}</td><td className="px-2">{member.sqlite ? "SQLite" : "文件"}</td><td className="whitespace-nowrap text-right">{member.size === null ? "原本不存在" : member.present ? humanBytes(member.size) : "缺失"}</td></tr>)}</tbody></table></div>
              </details>
              <form className="space-y-3 border-t pt-4" onSubmit={(event) => { event.preventDefault(); void run("restore"); }}>
                <h3 className="font-medium">恢复到隔离目录</h3>
                <p className="text-xs text-muted-foreground">这是删除涉及数据的恢复材料，包含共享数据库。不会回写原目录，也不保证直接续聊。</p>
                <label className="block space-y-1 text-sm"><span>已有父目录</span><Input value={parent} disabled={!!busy} onChange={(event) => setParent(event.target.value)} placeholder="输入服务所在电脑上的目录" /></label>
                <Button type="button" variant="outline" size="sm" disabled={!!busy} onClick={() => { void utility(async () => { const picked = await pickDirectoryPath({ title: "选择恢复父目录", defaultPath: parent || undefined }); if (picked && mounted.current) setParent(picked); }, ""); }}>选择父目录</Button>
                <label className="block space-y-1 text-sm"><span>新目录名称</span><Input value={folder} disabled={!!busy} onChange={(event) => setFolder(event.target.value)} /></label>
                {recoveryFolderError(folder) && <p className="text-xs text-destructive">{recoveryFolderError(folder)}</p>}
                {output && <p className="break-all text-xs">将创建：{output}</p>}
                <p className="text-xs text-muted-foreground">目标必须尚不存在；已有目录会被拒绝，不能合并或覆盖。</p>
                <Button type="submit" disabled={!!busy || !output || detail.status !== "unverified" || check?.ok === false}><RotateCcw className="mr-1 h-4 w-4" />校验并恢复</Button>
              </form>
              {restore && <div role={restore.ok ? "status" : "alert"} className="space-y-2 rounded-md border bg-muted/30 p-3 text-sm">
                <p className="font-medium">{restore.ok ? "隔离恢复完成" : "隔离恢复未完成"} · {dateText(restore.time)}</p>
                <p className="break-all text-xs">目标：{restore.output}</p>
                {restore.error && <p className="break-all text-destructive">{restore.error}</p>}
                {restore.ok && <><p className="text-xs">所有文件已回读校验，完成标记已写入。原会话目录保持不变。</p><Button variant="outline" size="sm" onClick={() => { void utility(() => api.revealCwd(restore.output), "已请求打开恢复目录"); }}>打开恢复目录</Button></>}
              </div>}
              {(history.verify || history.restore) && <details className="border-t pt-3"><summary className="cursor-pointer text-sm">最近操作记录（此浏览器）</summary>
                <p className="mt-2 text-xs text-muted-foreground">历史记录不代表文件当前完整；目录中的文件可能在操作后变化。</p>
                {(["verify", "restore"] as const).map((kind) => { const item = history[kind]; return item && <div key={kind} className="mt-2 space-y-1 text-xs">
                  <p>{kind === "verify" ? "上次校验" : "上次隔离恢复"}：{item.ok ? "成功" : "未完成"} · {dateText(item.time)}</p>
                  {item.output && <p className="break-all">目标：{item.output}</p>}
                  {item.error && <p className="break-all text-destructive">{item.error}</p>}
                  {kind === "restore" && item.ok && item.output && <Button size="sm" variant="outline" onClick={() => { void utility(() => api.revealCwd(item.output!), "已请求打开上次恢复目录"); }}>打开上次恢复目录</Button>}
                </div>; })}
              </details>}
            </>}
            {utilityMessage && <p role="status" className="break-all text-xs">{utilityMessage}</p>}
          </section>
        </div>}
      </div>
    </ScrollArea>
  </>;
}
