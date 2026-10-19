import { createContext, useContext, useMemo, useReducer } from "react";
import { SearchHighlightContext } from "./SearchHighlight";
import { resolveSearchDisclosure, toggleSearchDisclosure, type DisclosureState } from "@/lib/preview-search-disclosure";

export type DisclosureMemory = {
  manual: Map<string, boolean>;
  search: Map<string, DisclosureState["searchOverride"]>;
};

export const SearchDisclosureContext = createContext<(DisclosureMemory & { eventIndex: number }) | null>(null);

/** The dialog owns memory so filtering and virtual-row unmounts cannot erase it. */
export function useSearchDisclosure(section: string, content: string | object) {
  const memory = useContext(SearchDisclosureContext);
  const query = useContext(SearchHighlightContext).trim().toLowerCase();
  const [, refresh] = useReducer((version: number) => version + 1, 0);
  const matched = useMemo(() => !!query && (typeof content === "string" ? content : (JSON.stringify(content) ?? "")).toLowerCase().includes(query), [content, query]);
  const key = `${memory?.eventIndex}:${section}`;
  const state: DisclosureState = {
    manualOpen: memory?.manual.get(key) ?? false,
    searchOverride: memory?.search.get(key),
  };
  const open = resolveSearchDisclosure(state, query, matched);
  const toggle = () => {
    if (!memory) return;
    const next = toggleSearchDisclosure(state, query, matched);
    if (query) memory.search.set(key, next.searchOverride);
    else memory.manual.set(key, next.manualOpen);
    refresh();
  };
  return { open, toggle };
}
