import type { DeleteSnapshotSummary } from "./api";

export function deleteSnapshotLink(snapshotPath?: string): string {
  const name = snapshotPath?.replace(/\\/g, "/").split("/").filter(Boolean).pop();
  return `/codex/delete-snapshots${name ? `?snapshot=${encodeURIComponent(name)}` : ""}`;
}

export function deleteSnapshotStatus(status: DeleteSnapshotSummary["status"]): string {
  return { unverified: "尚未校验", incomplete: "文件不完整", unreadable: "无法读取" }[status];
}

export function recoveryFolderError(name: string): string | null {
  if (!name.trim()) return "请输入新目录名称";
  if (name !== name.trim() || name === "." || name === ".." || /[\\/:*?"<>|\u0000-\u001f]/.test(name)
    || name.endsWith(".") || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i.test(name)) {
    return "请使用单个有效目录名，不要填写路径或保留名称";
  }
  return null;
}

export type SnapshotActivity = { ok: boolean; time: string; output?: string; error?: string };
export type SnapshotActivities = { verify?: SnapshotActivity; restore?: SnapshotActivity };
const activityKey = (path: string) => `agentvault.delete-snapshot.activity.v1:${path}`;

export function readSnapshotActivities(storage: Pick<Storage, "getItem">, path: string): SnapshotActivities {
  try {
    const raw = storage.getItem(activityKey(path));
    if (!raw || raw.length > 32768) return {};
    const data = JSON.parse(raw) as SnapshotActivities;
    const result: SnapshotActivities = {};
    for (const kind of ["verify", "restore"] as const) {
      const item = data?.[kind];
      if (item && typeof item.ok === "boolean" && typeof item.time === "string" && Number.isFinite(Date.parse(item.time))
        && (item.output === undefined || typeof item.output === "string") && (item.error === undefined || typeof item.error === "string")) result[kind] = item;
    }
    return result;
  } catch { return {}; }
}

export function saveSnapshotActivity(storage: Pick<Storage, "getItem" | "setItem">, path: string, kind: keyof SnapshotActivities, item: SnapshotActivity): SnapshotActivities {
  const history = { ...readSnapshotActivities(storage, path), [kind]: item };
  storage.setItem(activityKey(path), JSON.stringify(history));
  return history;
}
