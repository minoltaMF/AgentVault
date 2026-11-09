import assert from "node:assert/strict";
import test from "node:test";
import { continuePreviewSearch } from "./preview-continuous-search";

type Event = { index: number; match?: boolean };
const matches = (event: Event) => event.match === true;
const isCancelled = () => false;
const onProgress = () => {};
const yieldControl = async () => {};

test("continuous search stops at the first matching page and selects its nearest later hit", async () => {
  let start = 200;
  const reads: number[] = [];
  const progress: number[][] = [];
  const result = await continuePreviewSearch<Event>({
    direction: "after", matches, isCancelled, yieldControl,
    nextPage: () => ({ start, count: 200 }),
    readPage: async (page) => {
      reads.push(page.start);
      start += 200;
      return page.start === 200 ? [{ index: 210 }] : [{ index: 450, match: true }, { index: 420, match: true }];
    },
    onProgress: (pages, events) => progress.push([pages, events]),
  });
  assert.deepEqual(reads, [200, 400]);
  assert.deepEqual(progress, [[1, 1], [2, 3]]);
  assert.deepEqual(result, { status: "found", hit: 420, pages: 2, events: 3 });
});

test("continuous earlier search chooses the greatest source index regardless of batch order", async () => {
  const result = await continuePreviewSearch<Event>({
    direction: "before", matches, isCancelled, onProgress,
    nextPage: () => ({ start: 400, count: 200 }),
    readPage: async () => [{ index: 410, match: true }, { index: 590, match: true }, { index: 500, match: true }],
  });
  assert.deepEqual(result, { status: "found", hit: 590, pages: 1, events: 3 });
});

test("filtered and empty pages continue until the caller confirms the boundary", async () => {
  let index = 0;
  const batches: Event[][] = [[{ index: 200 }], []];
  const result = await continuePreviewSearch<Event>({
    direction: "after", matches, isCancelled, onProgress, yieldControl,
    nextPage: () => index < batches.length ? { start: 200 + index * 200, count: 200 } : null,
    readPage: async () => batches[index++],
  });
  assert.deepEqual(result, { status: "boundary", hit: null, pages: 2, events: 1 });
});

test("an existing boundary or cancellation issues no reads", async () => {
  for (const cancelled of [false, true]) {
    const result = await continuePreviewSearch<Event>({
      direction: "before", matches, onProgress, isCancelled: () => cancelled,
      nextPage: () => null,
      readPage: async () => { assert.fail("must not read"); },
    });
    assert.deepEqual(result, { status: cancelled ? "cancelled" : "boundary", hit: null, pages: 0, events: 0 });
  }
});

test("read failure stops without discarding the count of accepted pages", async () => {
  for (const throws of [false, true]) {
    let start = 200;
    const result = await continuePreviewSearch<Event>({
      direction: "after", matches, isCancelled, onProgress, yieldControl,
      nextPage: () => ({ start, count: 200 }),
      readPage: async () => {
        if (start === 200) { start = 400; return [{ index: 210 }]; }
        if (throws) throw new Error("file unavailable");
        return null;
      },
    });
    assert.deepEqual(result, { status: "failed", hit: null, pages: 1, events: 1 });
  }
});

test("cancelling an in-flight page suppresses its hit and progress", async () => {
  let cancelled = false;
  let resolvePage!: (events: Event[]) => void;
  const page = new Promise<Event[]>((resolve) => { resolvePage = resolve; });
  const pending = continuePreviewSearch<Event>({
    direction: "after", matches, isCancelled: () => cancelled,
    nextPage: () => ({ start: 200, count: 200 }),
    readPage: async () => page,
    onProgress: () => { assert.fail("cancelled page must not publish progress"); },
  });
  cancelled = true;
  resolvePage([{ index: 201, match: true }]);
  assert.deepEqual(await pending, { status: "cancelled", hit: null, pages: 0, events: 0 });
});

test("cancellation between pages keeps progress and prevents the next request", async () => {
  let cancelled = false;
  let reads = 0;
  const result = await continuePreviewSearch<Event>({
    direction: "after", matches, isCancelled: () => cancelled, onProgress,
    nextPage: () => ({ start: 200 + reads * 200, count: 200 }),
    readPage: async () => { reads += 1; return [{ index: 210 }]; },
    yieldControl: async () => { cancelled = true; },
  });
  assert.equal(reads, 1);
  assert.deepEqual(result, { status: "cancelled", hit: null, pages: 1, events: 1 });
});

test("repeated or backwards page offsets stop without issuing another read", async () => {
  for (const direction of ["before", "after"] as const) {
    for (const nextStart of [400, direction === "before" ? 600 : 200]) {
      let reads = 0;
      const result = await continuePreviewSearch<Event>({
        direction, matches, isCancelled, onProgress, yieldControl,
        nextPage: () => ({ start: reads ? nextStart : 400, count: 200 }),
        readPage: async () => { reads += 1; return []; },
      });
      assert.equal(reads, 1);
      assert.deepEqual(result, { status: "failed", hit: null, pages: 1, events: 0 });
    }
  }
});

test("invalid page ranges fail before reading", async () => {
  for (const page of [{ start: -1, count: 200 }, { start: 0, count: 0 }, { start: 0.5, count: 200 }]) {
    const result = await continuePreviewSearch<Event>({
      direction: "after", matches, isCancelled, onProgress,
      nextPage: () => page,
      readPage: async () => { assert.fail("invalid range must not read"); },
    });
    assert.equal(result.status, "failed");
  }
});
