import assert from "node:assert/strict";
import test from "node:test";
import { previewSearchShortcut } from "./preview-search-shortcuts";

const preview = { inPreview: true, inSearch: false, editing: false };
const search = { ...preview, inSearch: true, editing: true };

test("find focuses search with exactly one platform modifier", () => {
  assert.equal(previewSearchShortcut({ key: "f", ctrlKey: true }, preview), "focus");
  assert.equal(previewSearchShortcut({ key: "F", metaKey: true }, search), "focus");
  for (const event of [{ key: "f" }, { key: "f", ctrlKey: true, metaKey: true }, { key: "f", ctrlKey: true, shiftKey: true }, { key: "f", metaKey: true, altKey: true }]) {
    assert.equal(previewSearchShortcut(event, search), null);
  }
});

test("Enter navigates messages or occurrences only inside search", () => {
  assert.equal(previewSearchShortcut({ key: "Enter" }, search), "next-message");
  assert.equal(previewSearchShortcut({ key: "Enter", shiftKey: true }, search), "previous-message");
  assert.equal(previewSearchShortcut({ key: "Enter", altKey: true }, search), "next-occurrence");
  assert.equal(previewSearchShortcut({ key: "Enter", altKey: true, shiftKey: true }, search), "previous-occurrence");
  assert.equal(previewSearchShortcut({ key: "Enter" }, preview), null);
  assert.equal(previewSearchShortcut({ key: "Enter", altKey: true }, { ...preview, editing: true }), null);
  for (const modifier of [{ ctrlKey: true }, { metaKey: true }, { altKey: true, ctrlKey: true }]) {
    assert.equal(previewSearchShortcut({ key: "Enter", ...modifier }, search), null);
  }
});

test("F3 navigates outside other editors and rejects extra modifiers", () => {
  for (const scope of [preview, search]) {
    assert.equal(previewSearchShortcut({ key: "F3" }, scope), "next-message");
    assert.equal(previewSearchShortcut({ key: "F3", shiftKey: true }, scope), "previous-message");
  }
  assert.equal(previewSearchShortcut({ key: "F3" }, { ...preview, editing: true }), null);
  for (const modifier of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }]) {
    assert.equal(previewSearchShortcut({ key: "F3", ...modifier }, preview), null);
  }
});

test("IME, handled events and nested surfaces never trigger preview search", () => {
  for (const event of [{ key: "Enter" }, { key: "F3" }, { key: "f", ctrlKey: true }]) {
    for (const blocked of [{ isComposing: true }, { keyCode: 229 }, { defaultPrevented: true }]) {
      assert.equal(previewSearchShortcut({ ...event, ...blocked }, search), null);
    }
    assert.equal(previewSearchShortcut(event, { ...search, inPreview: false }), null);
  }
  assert.equal(previewSearchShortcut({ key: "Escape" }, search), null);
});
