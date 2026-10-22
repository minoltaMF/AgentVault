import { readPositions, type ReadingPosition } from "./preview-reading-position";

export const SEARCH_STATE_KEY = "agentvault.preview-search.v1";
export const MAX_SEARCH_STATES = 100;
const MAX_STORAGE_LENGTH = 4_000_000;
type StorageLike = Pick<Storage, "getItem" | "setItem">;
export type SearchState = {
  key: string;
  query: string;
  position: ReadingPosition | null;
  occurrenceIndex: number;
  savedAt: number;
};

function normalize(value: unknown): SearchState | null {
  if (!value || typeof value !== "object") return null;
  const state = value as SearchState;
  if (typeof state.key !== "string" || !state.key || state.key.length > 32768
    || typeof state.query !== "string" || !state.query.trim() || state.query.length > 4096
    || !Number.isSafeInteger(state.occurrenceIndex) || state.occurrenceIndex < 0
    || !Number.isFinite(state.savedAt) || state.savedAt < 0) return null;
  let position: ReadingPosition | null = null;
  if (state.position !== null) {
    // Use the same signature and offset validation as ordinary reading restoration.
    const validated = readPositions({ getItem: () => JSON.stringify([state.position]), setItem() {} })[0];
    if (!validated || validated.key !== state.key) return null;
    const { key, eventIndex, eventOffset, signature, newestFirst, onlyMessages, savedAt } = validated;
    position = { key, eventIndex, eventOffset, signature, newestFirst, onlyMessages, savedAt };
  }
  // Whitelist fields so accidental transcript properties are never persisted.
  return { key: state.key, query: state.query, position, occurrenceIndex: state.occurrenceIndex, savedAt: state.savedAt };
}

function latest(states: SearchState[]): SearchState[] {
  const seen = new Set<string>();
  return states.sort((a, b) => b.savedAt - a.savedAt)
    .filter((state) => {
      if (seen.has(state.key)) return false;
      seen.add(state.key);
      return true;
    }).slice(0, MAX_SEARCH_STATES);
}

function parse(raw: string | null): SearchState[] {
  if (!raw || raw.length > MAX_STORAGE_LENGTH) return [];
  try {
    const values: unknown = JSON.parse(raw);
    return Array.isArray(values) ? latest(values.map(normalize).filter((value): value is SearchState => value !== null)) : [];
  } catch { return []; }
}

export function readSearchStates(storage: StorageLike): SearchState[] {
  try { return parse(storage.getItem(SEARCH_STATE_KEY)); } catch { return []; }
}

function write(storage: StorageLike, states: SearchState[]): boolean {
  const raw = JSON.stringify(states);
  if (raw.length > MAX_STORAGE_LENGTH) return false;
  storage.setItem(SEARCH_STATE_KEY, raw);
  return true;
}

export function saveSearchState(storage: StorageLike, state: SearchState): boolean {
  try {
    const valid = normalize(state);
    if (!valid) return false;
    return write(storage, latest([valid, ...parse(storage.getItem(SEARCH_STATE_KEY)).filter((item) => item.key !== valid.key)]));
  } catch { return false; }
}

export function removeSearchState(storage: StorageLike, key: string): boolean {
  try { return write(storage, parse(storage.getItem(SEARCH_STATE_KEY)).filter((state) => state.key !== key)); }
  catch { return false; }
}
