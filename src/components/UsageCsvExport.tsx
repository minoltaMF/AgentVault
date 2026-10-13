import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { saveFilePath } from "@/lib/dialog";
import { isTauriRuntime } from "@/lib/runtime";
import { buildUsageCsv } from "@/lib/usage-csv";
import type { PriceBook, UsageDateRange, UsageStatus } from "@/lib/session-usage";
import { validateUsageDateRange } from "@/lib/session-usage";

export function UsageCsvExport({ status, prices, range, period, error }: {
  status: UsageStatus | null; prices: PriceBook; range: UsageDateRange;
  period: { days: 7 | 30; now: Date }; error: string;
}) {
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState("");
  const [failed, setFailed] = useState(false);
  const dateError = validateUsageDateRange(range);
  async function exportCsv() {
    if (!status || saving || dateError || status.state === "running") return;
    setSaving(true); setMessage(""); setFailed(false);
    try {
      // Capture the visible scope and period before awaiting the save dialog.
      const generatedAt = new Date();
      const csv = buildUsageCsv({ status, prices, range, period, error, generatedAt });
      const name = `agentvault-usage-${generatedAt.toISOString().replace(/[:.]/g, "-")}.csv`;
      if (isTauriRuntime()) {
        const path = await saveFilePath({ title: "导出用量 CSV（请选择新文件）", defaultPath: name, filters: [{ name: "CSV", extensions: ["csv"] }] });
        if (!path) return;
        await invoke("save_usage_csv", { outPath: path, csv });
        setMessage("CSV 已保存。");
      } else {
        const url = URL.createObjectURL(new Blob([csv], { type: "text/csv;charset=utf-8" }));
        const link = document.createElement("a");
        try {
          link.href = url; link.download = name;
          document.body.appendChild(link); link.click();
          setMessage("已请求浏览器下载 CSV，请查看下载列表。");
        } finally {
          link.remove();
          setTimeout(() => URL.revokeObjectURL(url), 30_000);
        }
      }
    } catch (failure) {
      setFailed(true); setMessage(`导出失败：${String(failure)}`);
    } finally { setSaving(false); }
  }
  return <section aria-label="用量 CSV 导出" className="space-y-2 rounded-md border p-3">
    <Button size="sm" variant="outline" disabled={!status || status.state === "running" || !!dateError || saving} onClick={() => void exportCsv()}>{saving ? "正在导出…" : "导出用量 CSV"}</Button>
    <p className="text-xs text-muted-foreground">导出当前日期范围汇总、全部每日数据和所选 {period.days} 天周期对比，包含未知费用与完整性说明；不受分页限制，不包含对话正文。分析结束或取消后可导出，部分结果会明确标注。桌面版请选择新文件，不覆盖已有文件。</p>
    {!!dateError && <p className="text-xs text-destructive">请先修正日期范围。</p>}
    {!!message && <p role={failed ? "alert" : "status"} className="break-all text-xs">{message}</p>}
  </section>;
}
