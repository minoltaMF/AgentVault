import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { pickFilePath, saveFilePath } from "@/lib/dialog";
import { isTauriRuntime } from "@/lib/runtime";
import { rateFields, type PriceBook, type UsageSession } from "@/lib/session-usage";
import { applyPriceDrafts, maxPriceImportBytes, mergePriceImport, parsePriceImport, previewPriceImport, serializePriceBook, usagePriceCatalog, type PriceDrafts } from "@/lib/usage-prices";

const labels = { input: "未缓存输入", output: "输出", cache_read: "缓存读取", cache_write_5m: "缓存写入（5 分钟）", cache_write_1h: "缓存写入（1 小时）" };
export function UsagePriceManager({ sessions, prices, onSave, onDirtyChange }: { sessions: UsageSession[]; prices: PriceBook; onSave: (next: PriceBook) => void; onDirtyChange: (dirty: boolean) => void }) {
  const [drafts, setDrafts] = useState<PriceDrafts>({});
  const [missingOnly, setMissingOnly] = useState(true);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [incoming, setIncoming] = useState<PriceBook | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [message, setMessage] = useState("");
  const [failed, setFailed] = useState(false);
  const catalog = useMemo(() => usagePriceCatalog(sessions, prices), [sessions, prices]);
  const missingCount = catalog.filter(row => row.missingFields.length > 0).length;
  const filtered = useMemo(() => catalog.filter(row => (!missingOnly || row.missingFields.length > 0) && `${row.provider} ${row.model}`.toLowerCase().includes(search.trim().toLowerCase())), [catalog, missingOnly, search]);
  const pages = Math.max(1, Math.ceil(filtered.length / 10));
  const currentPage = Math.min(page, pages - 1);
  const draftCount = Object.keys(drafts).length;
  useEffect(() => { onDirtyChange(draftCount > 0 || incoming !== null); return () => onDirtyChange(false); }, [draftCount, incoming, onDirtyChange]);
  const previewResult = useMemo(() => {
    try { return { preview: incoming ? previewPriceImport(prices, incoming) : null, error: "" }; }
    catch (failure) { return { preview: null, error: `无法合并配置：${String(failure)}` }; }
  }, [prices, incoming]);
  const preview = previewResult.preview;
  const locked = busy || incoming !== null;
  function report(text: string, failure = false) { setMessage(text); setFailed(failure); }
  function saveDrafts() {
    if (busyRef.current || incoming || !draftCount) return;
    try { onSave(applyPriceDrafts(prices, drafts)); setDrafts({}); report("全部单价修改已保存，费用已重新计算。"); }
    catch (failure) { report(`保存失败：${String(failure)}；草稿已保留。`, true); }
  }
  async function importPrices(file?: File) {
    if (busyRef.current) return;
    if (draftCount) { report("请先保存或放弃未保存的单价修改，再导入配置。", true); return; }
    busyRef.current = true; setBusy(true); report("");
    try {
      let raw: string;
      if (isTauriRuntime()) {
        const path = await pickFilePath({ title: "导入模型单价配置", filters: [{ name: "JSON", extensions: ["json"] }] });
        if (!path) { report("已取消导入。"); return; }
        raw = await invoke<string>("read_usage_prices", { path });
      } else {
        if (!file) return;
        if (file.size > maxPriceImportBytes) throw new Error("单价配置文件不能超过 1 MiB");
        raw = await file.text();
      }
      const parsed = parsePriceImport(raw);
      previewPriceImport(prices, parsed);
      setIncoming(parsed); report("请核对合并预览，再应用配置。");
    } catch (failure) { report(`导入失败：${String(failure)}`, true); }
    finally { busyRef.current = false; setBusy(false); }
  }
  function merge() {
    if (busyRef.current || !incoming || draftCount) return;
    try { onSave(mergePriceImport(prices, incoming)); setIncoming(null); report("配置已合并保存，导入中未提供的字段保持不变。"); }
    catch (failure) { report(`合并失败：${String(failure)}；预览已保留。`, true); }
  }
  async function exportPrices() {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true); report("");
    try {
      const json = serializePriceBook(prices);
      const name = `agentvault-prices-${new Date().toISOString().replace(/[:.]/g, "-")}.json`;
      if (isTauriRuntime()) {
        const path = await saveFilePath({ title: "导出模型单价（请选择新文件）", defaultPath: name, filters: [{ name: "JSON", extensions: ["json"] }] });
        if (!path) { report("已取消导出。"); return; }
        await invoke("save_usage_prices", { outPath: path, json }); report("已保存单价配置。");
      } else {
        const url = URL.createObjectURL(new Blob([json], { type: "application/json;charset=utf-8" }));
        const link = document.createElement("a");
        try { link.href = url; link.download = name; document.body.appendChild(link); link.click(); report("已请求浏览器下载已保存配置，请查看下载列表。"); }
        finally { link.remove(); setTimeout(() => URL.revokeObjectURL(url), 30_000); }
      }
    } catch (failure) { report(`导出失败：${String(failure)}`, true); }
    finally { busyRef.current = false; setBusy(false); }
  }
  return <section aria-label="模型单价管理" className="min-w-0 space-y-3 rounded-md border p-3">
    <h3 className="text-sm font-medium">模型单价管理</h3>
    <p className="text-xs text-muted-foreground">共 {catalog.length} 个模型，其中 {missingCount} 个模型缺少当前用量所需单价。单位为 USD / 百万 Token，空值表示未知，0 表示免费。未编辑的字段保持不变；切换筛选或分页会保留草稿。</p>
    <div className="flex flex-wrap items-center gap-2">
      <Button size="sm" variant={missingOnly ? "default" : "outline"} aria-pressed={missingOnly} onClick={() => { setMissingOnly(true); setPage(0); }}>仅缺单价（{missingCount}）</Button>
      <Button size="sm" variant={!missingOnly ? "default" : "outline"} aria-pressed={!missingOnly} onClick={() => { setMissingOnly(false); setPage(0); }}>全部模型（{catalog.length}）</Button>
      <Input className="w-full sm:w-60" aria-label="搜索单价模型或来源" placeholder="搜索模型或来源" value={search} onChange={event => { setSearch(event.target.value); setPage(0); }} />
    </div>
    {filtered.slice(currentPage * 10, (currentPage + 1) * 10).map(row => <fieldset key={row.key} disabled={locked} className="min-w-0 space-y-2 rounded border p-2">
      <legend className="max-w-full break-all px-1 text-xs font-medium">{row.provider} · {row.model || "未知模型"}</legend>
      <p className="text-xs text-muted-foreground">{row.sessionCount} 条会话{row.missingFields.length ? ` · 缺少：${row.missingFields.map(field => labels[field]).join("、")}` : " · 当前用量无缺失单价"}{row.invalidUsage ? " · 存在无效用量，无法完整判断费用" : ""}{drafts[row.key] ? " · 有未保存修改" : ""}</p>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">{rateFields.map(field => <label key={field} className="min-w-0 space-y-1 text-xs"><span>{labels[field]}</span><Input aria-label={`${row.provider} ${row.model} ${labels[field]}单价`} inputMode="decimal" placeholder="未知" value={drafts[row.key]?.[field] ?? (row.rates[field] === undefined ? "" : String(row.rates[field]))} onChange={event => { const value = event.target.value; setDrafts(previous => ({ ...previous, [row.key]: { ...previous[row.key], [field]: value } })); }} /></label>)}</div>
    </fieldset>)}
    {!filtered.length && <p className="text-xs text-muted-foreground">当前筛选没有模型。</p>}
    <div className="flex flex-wrap items-center gap-2 text-xs"><Button size="sm" variant="outline" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>上一页模型</Button><span>第 {currentPage + 1} / {pages} 页 · {filtered.length} 个模型</span><Button size="sm" variant="outline" disabled={currentPage + 1 >= pages} onClick={() => setPage(currentPage + 1)}>下一页模型</Button></div>
    <div className="flex flex-wrap gap-2"><Button size="sm" disabled={locked || !draftCount} onClick={saveDrafts}>保存全部修改（{draftCount} 个模型）</Button><Button size="sm" variant="outline" disabled={locked || !draftCount} onClick={() => { setDrafts({}); report("已放弃未保存的修改。"); }}>放弃编辑</Button></div>
    <div className="flex flex-wrap items-center gap-2">
      {isTauriRuntime() ? <Button size="sm" variant="outline" disabled={locked || !!draftCount} onClick={() => void importPrices()}>导入单价 JSON</Button> : <label className="min-w-0 space-y-1 text-xs"><span>导入单价 JSON（最多 1 MiB）</span><Input type="file" accept=".json,application/json" aria-label="导入单价 JSON" disabled={locked || !!draftCount} onChange={event => { const file = event.target.files?.[0]; event.target.value = ""; if (file) void importPrices(file); }} /></label>}
      <Button size="sm" variant="outline" disabled={busy} onClick={() => void exportPrices()}>导出已保存单价 JSON</Button>
    </div>
    <p className="text-xs text-muted-foreground">导入前请保存或放弃草稿；导入按来源和模型合并，不删除未提供的字段。导出不含未保存草稿、会话路径或正文。桌面导出请选择新文件。</p>
    {preview && <div className="space-y-2 rounded border p-2 text-xs" aria-label="单价导入预览"><p>新增模型 {preview.addedModels} 个；新增单价字段 {preview.addedFields} 个；覆盖单价字段 {preview.overwrittenFields} 个；保持原值 {preview.unchangedFields} 个。</p><div className="flex flex-wrap gap-2"><Button size="sm" disabled={busy} onClick={merge}>应用合并</Button><Button size="sm" variant="outline" disabled={busy} onClick={() => { setIncoming(null); report("已取消导入预览，配置未修改。"); }}>取消预览</Button></div></div>}
    {!!previewResult.error && <div role="alert" className="text-xs text-destructive">{previewResult.error}<Button size="sm" variant="outline" onClick={() => setIncoming(null)}>取消预览</Button></div>}
    {busy && <p role="status" className="text-xs">正在处理单价配置…</p>}
    {!!message && <p role={failed ? "alert" : "status"} className="break-all text-xs">{message}</p>}
  </section>;
}
