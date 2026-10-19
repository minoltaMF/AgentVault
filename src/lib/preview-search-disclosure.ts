export type DisclosureState = {
  manualOpen: boolean;
  searchOverride?: { query: string; open: boolean };
};

export function resolveSearchDisclosure(state: DisclosureState, query: string, matched: boolean): boolean {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) return state.manualOpen;
  if (state.searchOverride?.query === normalizedQuery) return state.searchOverride.open;
  return matched || state.manualOpen;
}

export function toggleSearchDisclosure(state: DisclosureState, query: string, matched: boolean): DisclosureState {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) return { manualOpen: !state.manualOpen };
  return {
    manualOpen: state.manualOpen,
    searchOverride: { query: normalizedQuery, open: !resolveSearchDisclosure(state, normalizedQuery, matched) },
  };
}
