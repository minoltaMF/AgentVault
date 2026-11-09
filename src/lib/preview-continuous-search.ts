type SearchPage = { start: number; count: number };

export type ContinuousSearchResult = {
  status: "found" | "boundary" | "cancelled" | "failed";
  hit: number | null;
  pages: number;
  events: number;
};

/** Read serially from the loaded boundary; matching and page acceptance stay with the caller. */
export async function continuePreviewSearch<T extends { index: number }>(options: {
  direction: "before" | "after";
  nextPage: () => SearchPage | null;
  readPage: (page: SearchPage) => Promise<readonly T[] | null>;
  matches: (event: T) => boolean;
  isCancelled: () => boolean;
  onProgress: (pages: number, events: number) => void;
  yieldControl?: () => Promise<void>;
}): Promise<ContinuousSearchResult> {
  let pages = 0;
  let events = 0;
  let previousStart: number | null = null;
  const result = (status: ContinuousSearchResult["status"], hit: number | null = null): ContinuousSearchResult =>
    ({ status, hit, pages, events });
  const yieldControl = options.yieldControl ?? (() => new Promise<void>((resolve) => setTimeout(resolve, 0)));

  while (true) {
    if (options.isCancelled()) return result("cancelled");
    const page = options.nextPage();
    if (!page) return result("boundary");
    if (!Number.isSafeInteger(page.start) || page.start < 0 || !Number.isSafeInteger(page.count) || page.count <= 0
      || (previousStart !== null && (options.direction === "after" ? page.start <= previousStart : page.start >= previousStart))) {
      return result("failed");
    }
    previousStart = page.start;
    if (options.isCancelled()) return result("cancelled");

    let batch: readonly T[] | null;
    try {
      batch = await options.readPage(page);
    } catch {
      return result(options.isCancelled() ? "cancelled" : "failed");
    }
    if (options.isCancelled()) return result("cancelled");
    if (batch === null) return result("failed");
    pages += 1;
    events += batch.length;
    options.onProgress(pages, events);
    if (options.isCancelled()) return result("cancelled");

    let hit: number | null = null;
    for (const event of batch) {
      if (options.matches(event) && (hit === null || (options.direction === "before" ? event.index > hit : event.index < hit))) {
        hit = event.index;
      }
    }
    if (hit !== null) return result("found", hit);
    await yieldControl();
  }
}
