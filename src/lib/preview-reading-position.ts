import type { PreviewEvent } from "./api";

export const READING_POSITION_KEY = "agentvault.preview-reading.v1";
export const MAX_READING_POSITIONS = 100;
export type ReadingPosition = {
  key: string; eventIndex: number; eventOffset: number; signature: string;
  newestFirst: boolean; onlyMessages: boolean; savedAt: number;
};
type StorageLike = Pick<Storage, "getItem" | "setItem">;
export const readingPositionKey = (provider: string, path: string) => JSON.stringify([provider, path]);

// A change detector, not a security fingerprint. Never persist transcript text.
export function readingEventSignature(event: PreviewEvent): string {
  const value = JSON.stringify([event.timestamp, event.role, event.kind, event.raw]);
  let hash = 2166136261;
  for (let i = 0; i < value.length; i++) hash = Math.imul(hash ^ value.charCodeAt(i), 16777619);
  return `${value.length}:${hash >>> 0}`;
}
function valid(value: unknown): value is ReadingPosition {
  if (!value || typeof value !== "object") return false;
  const p = value as ReadingPosition;
  return typeof p.key === "string" && p.key.length <= 32768
    && Number.isSafeInteger(p.eventIndex) && p.eventIndex >= 0
    && Number.isSafeInteger(p.eventOffset) && p.eventOffset >= 0
    && typeof p.signature === "string" && /^\d+:\d+$/.test(p.signature)
    && typeof p.newestFirst === "boolean" && typeof p.onlyMessages === "boolean"
    && Number.isFinite(p.savedAt) && p.savedAt >= 0;
}
export function readPositions(storage: StorageLike): ReadingPosition[] {
  try {
    const raw = storage.getItem(READING_POSITION_KEY);
    if (!raw || raw.length > 4_000_000) return [];
    const data: unknown = JSON.parse(raw);
    return Array.isArray(data) ? data.filter(valid).sort((a, b) => b.savedAt - a.savedAt).slice(0, MAX_READING_POSITIONS) : [];
  } catch { return []; }
}
export function saveReadingPosition(storage: StorageLike, position: ReadingPosition): boolean {
  if (!valid(position)) return false;
  try {
    const positions = [position, ...readPositions(storage).filter((p) => p.key !== position.key)]
      .slice(0, MAX_READING_POSITIONS);
    storage.setItem(READING_POSITION_KEY, JSON.stringify(positions));
    return true;
  } catch { return false; }
}
export function matchesReadingPosition(events: readonly PreviewEvent[], start: number, position: ReadingPosition): boolean {
  const event = events[position.eventOffset - start];
  return !!event && event.index === position.eventIndex && readingEventSignature(event) === position.signature;
}
