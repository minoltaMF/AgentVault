import { useCallback, useEffect, useRef, useState } from "react";
import { api, type SessionSummary, type WorkbenchScanStatus } from "@/lib/api";
import { requestGeneration, type WorkbenchProvider } from "@/lib/allSessions";

export function useWorkbenchSource(provider: WorkbenchProvider, root: string, codexRoot: string) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [state, setState] = useState<"idle" | "loading" | "ready" | "error" | "cancelled" | "interrupted">("idle");
  const [error, setError] = useState("");
  const [checkedAt, setCheckedAt] = useState<string | null>(null);
  const [progress, setProgress] = useState<WorkbenchScanStatus | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const requests = useRef(requestGeneration());
  const inFlight = useRef(false);
  const job = useRef<number | null>(null);
  const cancelRequested = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryPoll = useRef<(() => void) | null>(null);
  useEffect(() => () => {
    requests.current.invalidate();
    if (timer.current) clearTimeout(timer.current);
    if (job.current !== null) void api.cancelWorkbenchScan(job.current).catch(() => {});
  }, []);

  const refresh = useCallback(async () => {
    if (!root || inFlight.current) return;
    inFlight.current = true; cancelRequested.current = false;
    const request = requests.current.next();
    const current = () => requests.current.current(request);
    setState("loading"); setError(""); setProgress(null); setCancelling(false);
    try {
      const started = await api.startWorkbenchScan(provider, codexRoot, provider === "claude" ? root : "");
      // A start response can arrive after navigation or a source-scope remount.
      if (!current()) { void api.cancelWorkbenchScan(started.job_id).catch(() => {}); return; }
      job.current = started.job_id;
      const poll = async () => {
        if (!current()) return;
        retryPoll.current = null;
        try {
          const status = await api.workbenchScanStatus(started.job_id);
          if (!current()) return;
          setProgress(status);
          if (status.state === "running") {
            timer.current = setTimeout(() => { void poll(); }, 500);
            return;
          }
          job.current = null; inFlight.current = false; setCancelling(false); setError("");
          if (status.state === "completed") {
            setSessions(status.results); setCheckedAt(new Date().toISOString()); setState("ready");
          } else if (status.state === "cancelled") setState("cancelled");
          else { setState("error"); setError(status.error || "扫描失败"); }
        } catch (error) {
          if (!current()) return;
          // A failed status request is not evidence that the worker stopped.
          setState("interrupted"); setError(`无法查询扫描状态：${error instanceof Error ? error.message : String(error)}`);
          retryPoll.current = () => { setState("loading"); setError(""); void poll(); };
        }
      };
      if (cancelRequested.current) {
        try { await api.cancelWorkbenchScan(started.job_id); }
        catch (error) { if (current()) { setCancelling(false); setError(`停止请求失败：${String(error)}`); } }
      }
      void poll();
    } catch (error) {
      if (current()) { inFlight.current = false; setCancelling(false); setError(error instanceof Error ? error.message : String(error)); setState("error"); }
    }
  }, [provider, root, codexRoot]);

  const cancel = async () => {
    if (!inFlight.current) return;
    cancelRequested.current = true; setCancelling(true);
    const activeJob = job.current;
    if (activeJob === null) return; // Cancel once start returns its job ID.
    try {
      await api.cancelWorkbenchScan(activeJob);
      if (job.current === activeJob) retryPoll.current?.();
    } catch (error) {
      if (job.current === activeJob) { setCancelling(false); setError(`停止请求失败：${error instanceof Error ? error.message : String(error)}`); }
    }
  };
  return { sessions, state, error, checkedAt, progress, cancelling, refresh, cancel, retry: () => retryPoll.current?.() };
}
