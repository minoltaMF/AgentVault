import assert from "node:assert/strict";
import test from "node:test";
import { resolveSearchDisclosure, toggleSearchDisclosure, type DisclosureState } from "./preview-search-disclosure";

test("matching search opens collapsed content without changing the manual state", () => {
  const state: DisclosureState = { manualOpen: false };
  assert.equal(resolveSearchDisclosure(state, "needle", true), true);
  assert.equal(resolveSearchDisclosure(state, "needle", false), false);
  assert.equal(resolveSearchDisclosure(state, "", true), false);
  assert.deepEqual(state, { manualOpen: false });
});

test("search-time manual collapse survives the same normalized query and clears visually with search", () => {
  const state = toggleSearchDisclosure({ manualOpen: false }, " Needle ", true);
  assert.deepEqual(state, { manualOpen: false, searchOverride: { query: "needle", open: false } });
  assert.equal(resolveSearchDisclosure(state, "NEEDLE", true), false);
  assert.equal(resolveSearchDisclosure(state, " needle ", true), false);
  assert.equal(resolveSearchDisclosure(state, "   ", true), false);
  assert.equal(resolveSearchDisclosure(toggleSearchDisclosure(state, "needle", true), "needle", true), true);
});

test("switching keywords ignores the previous override and follows the new match", () => {
  const state = toggleSearchDisclosure({ manualOpen: false }, "first", true);
  assert.equal(resolveSearchDisclosure(state, "second", true), true);
  assert.equal(resolveSearchDisclosure(state, "second", false), false);
  const opened = toggleSearchDisclosure(state, "second", false);
  assert.equal(resolveSearchDisclosure(opened, "second", false), true);
  assert.equal(resolveSearchDisclosure(opened, "first", false), false);
});

test("search-time toggles preserve originally open content after clearing or switching", () => {
  const state = toggleSearchDisclosure({ manualOpen: true }, "needle", true);
  assert.equal(resolveSearchDisclosure(state, "needle", true), false);
  assert.equal(resolveSearchDisclosure(state, "", false), true);
  assert.equal(resolveSearchDisclosure(state, "other", false), true);
  assert.equal(state.manualOpen, true);
});

test("ordinary toggles update manual state and discard stale search overrides", () => {
  const state: DisclosureState = { manualOpen: true, searchOverride: { query: "needle", open: true } };
  const closed = toggleSearchDisclosure(state, " ", true);
  assert.deepEqual(closed, { manualOpen: false });
  assert.deepEqual(toggleSearchDisclosure(closed, "", false), { manualOpen: true });
  assert.deepEqual(state, { manualOpen: true, searchOverride: { query: "needle", open: true } });
});

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SearchDisclosureContext, useSearchDisclosure, type DisclosureMemory } from "../components/SearchDisclosure";
import { SearchHighlightContext } from "../components/SearchHighlight";

test("disclosure hook reveals matching bodies and restores manual memory across remounts", () => {
  const memory: DisclosureMemory = {manual: new Map(), search: new Map()};
  function Body() {
    const {open} = useSearchDisclosure("raw", {nested: "Needle"});
    return createElement("button", {"aria-expanded": open}, "details");
  }
  const render = (query: string, index = 4, search = memory.search) => renderToStaticMarkup(
    createElement(SearchHighlightContext.Provider, {value: query},
      createElement(SearchDisclosureContext.Provider, {value: {...memory, search, eventIndex: index}}, createElement(Body))));
  assert.match(render(" NEEDLE "), /aria-expanded="true"/);
  assert.match(render("absent"), /aria-expanded="false"/);
  memory.search.set("4:raw", {query: "needle", open: false});
  assert.match(render("needle"), /aria-expanded="false"/);
  assert.match(render("needle", 5), /aria-expanded="true"/);
  assert.match(render("needle", 4, new Map()), /aria-expanded="true"/);
  memory.manual.set("4:raw", true);
  assert.match(render(""), /aria-expanded="true"/);
  assert.match(render("", 5), /aria-expanded="false"/);
});
