import { READING_POSITION_KEY, readPositions } from "./preview-reading-position";
import { SEARCH_STATE_KEY, readSearchStates } from "./preview-search-state";

export const PREVIEW_MEMORY_KEY = "agentvault.preview-memory.v1";
export type MemoryPolicy = { enabled: boolean; revision: number };
type StorageLike = Pick<Storage, "getItem" | "setItem">;

function policy(storage: StorageLike): MemoryPolicy {
  const raw = storage.getItem(PREVIEW_MEMORY_KEY);
  if (raw === null) return { enabled: true, revision: 0 };
  const value = JSON.parse(raw) as MemoryPolicy | null;
  if (!value || typeof value.enabled !== "boolean" || !Number.isSafeInteger(value.revision) || value.revision < 0) {
    throw new Error("Invalid preview memory policy");
  }
  return { enabled: value.enabled, revision: value.revision };
}

export function readMemoryPolicy(storage: StorageLike): MemoryPolicy {
  try { return policy(storage); }
  catch { return { enabled: false, revision: 0 }; }
}

function advance(storage: StorageLike, enabled?: boolean): void {
  const current = policy(storage);
  if (current.revision === Number.MAX_SAFE_INTEGER) throw new Error("Memory revision exhausted");
  storage.setItem(PREVIEW_MEMORY_KEY, JSON.stringify({ enabled: enabled ?? current.enabled, revision: current.revision + 1 }));
}

export function setMemoryEnabled(storage: StorageLike, enabled: boolean): boolean {
  try { advance(storage, enabled); return true; }
  catch { return false; }
}

function checkedRecords<T>(storage: StorageLike, key: string, read: (storage: StorageLike) => T[]): T[] {
  // Read outside the forgiving restoration helpers: a failed read must never become an empty overwrite.
  const raw = storage.getItem(key);
  if (raw === null) return [];
  if (raw.length > 4_000_000) throw new Error("Oversized preview memory");
  const values: unknown = JSON.parse(raw);
  if (!Array.isArray(values)) throw new Error("Invalid preview memory");
  const records = read({ getItem: () => raw, setItem() {} });
  if (records.length !== values.length) throw new Error("Invalid preview memory records");
  return records;
}

export function clearPreviewMemories(storage: StorageLike, key?: string): boolean {
  try {
    // Invalidate open views before any deletion, including a partially failed deletion.
    advance(storage);
    const positions = key === undefined ? [] : checkedRecords(storage, READING_POSITION_KEY, readPositions).filter((item) => item.key !== key);
    const searches = key === undefined ? [] : checkedRecords(storage, SEARCH_STATE_KEY, readSearchStates).filter((item) => item.key !== key);
    storage.setItem(READING_POSITION_KEY, JSON.stringify(positions));
    storage.setItem(SEARCH_STATE_KEY, JSON.stringify(searches));
    return true;
  } catch { return false; }
}
