import type { PreviewEvent } from "./api";
import { previewEventSearchText } from "./previewEvent";

type Eligibility = (event: PreviewEvent) => boolean;
type Match = { eligible: boolean; matches: boolean };

/** Events are immutable API snapshots. Cache only the current query/filter pair;
 * object identity prevents a reread with the same source index reusing stale results.
 * Weak keys do not retain discarded pages or duplicate the existing text cache. */
export function createPreviewSearchProjection() {
  let currentQuery = "";
  let currentEligibility: Eligibility | undefined;
  let cache = new WeakMap<PreviewEvent, Match>();

  const prepare = (query: string, isEligible: Eligibility) => {
    if (currentQuery !== query || currentEligibility !== isEligible) {
      currentQuery = query;
      currentEligibility = isEligible;
      cache = new WeakMap();
    }
  };
  const classify = (event: PreviewEvent, query: string, isEligible: Eligibility): Match => {
    const cached = cache.get(event);
    if (cached) return cached;
    const eligible = isEligible(event);
    const result = { eligible, matches: eligible && (!query || previewEventSearchText(event).includes(query)) };
    cache.set(event, result);
    return result;
  };

  return {
    matches(event: PreviewEvent, query: string, isEligible: Eligibility): boolean {
      prepare(query, isEligible);
      return classify(event, query, isEligible).matches;
    },
    select(events: readonly PreviewEvent[], query: string, isEligible: Eligibility) {
      prepare(query, isEligible);
      const eligibleEvents: PreviewEvent[] = [];
      const eligibleIndices: number[] = [];
      const matchingEvents: PreviewEvent[] = [];
      for (const event of events) {
        const result = classify(event, query, isEligible);
        if (result.eligible) { eligibleEvents.push(event); eligibleIndices.push(event.index); }
        if (result.matches) matchingEvents.push(event);
      }
      return { eligibleEvents, eligibleIndices, matchingEvents };
    },
  };
}
