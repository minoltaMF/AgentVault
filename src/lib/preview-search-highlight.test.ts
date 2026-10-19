import assert from "node:assert/strict";
import test from "node:test";
import { findPreviewSearchRanges, rehypeSearchHighlight } from "./preview-search-highlight";

test("literal search trims queries, preserves original offsets and ignores regex syntax", () => {
  assert.deepEqual(findPreviewSearchRanges("Aa aA [x].* 中文😀中文", " aa "), [{start:0,end:2},{start:3,end:5}]);
  assert.deepEqual(findPreviewSearchRanges("[x].* [x].*", "[x].*"), [{start:0,end:5},{start:6,end:11}]);
  assert.deepEqual(findPreviewSearchRanges("中文😀中文", "😀中"), [{start:2,end:5}]);
  assert.deepEqual(findPreviewSearchRanges("abc", "  "), []);
  assert.deepEqual(findPreviewSearchRanges("aaaa", "aa"), [{start:0,end:2},{start:2,end:4}]);
});
test("Unicode lowercase expansion cannot shift or split original characters", () => {
  assert.deepEqual(findPreviewSearchRanges("İ Xİx", "x"), [{start:2,end:3},{start:4,end:5}]);
  assert.deepEqual(findPreviewSearchRanges("İİ", "i"), [{start:0,end:1},{start:1,end:2}]);
  assert.deepEqual(findPreviewSearchRanges("ΟΣ", "ος"), [{start:0,end:2}]);
});
const text = (value:string) => ({type:"text", value});
const element = (tagName:string, children:any[], properties:any={}) => ({type:"element", tagName, properties, children});
const marks = (node:any):any[] => node.tagName === "mark" ? [node] : (node.children ?? []).flatMap(marks);
const contents = (node:any):string => node.type === "text" ? node.value : (node.children ?? []).map(contents).join("");
test("rehype highlights across inline formatting while preserving links and code", () => {
  const link = element("a", [text("World")], {href:"https://example.test/HelloWorld"});
  const tree = {type:"root",children:[element("p", [text("He"),element("strong",[text("llo")]),link]), element("pre",[element("code",[text("HELLOWORLD")])])]};
  rehypeSearchHighlight({query:"helloworld"})(tree);
  assert.equal(contents(tree), "HelloWorldHELLOWORLD");
  assert.equal(marks(tree).length, 4);
  assert.equal(link.properties.href, "https://example.test/HelloWorld");
  assert.deepEqual(marks(tree)[0].properties.className,["preview-search-highlight"]);
});
test("rehype never matches across blocks, breaks or non-visible content", () => {
  const tree = {type:"root",children:[element("p",[text("foo")]),element("p",[text("bar")]),element("p",[text("foo"),element("br",[]),text("bar")]),element("script",[text("foobar")]),element("p",[text("foo"),element("img",[],{alt:"foobar"}),text("bar")])]};
  rehypeSearchHighlight({query:"foobar"})(tree);
  assert.equal(marks(tree).length,0);
});
test("occurrence identifiers group inline fragments and remain distinct across paragraphs", () => {
  const fixture = () => ({type: "root", children: [
    element("p", [text("he"), element("strong", [text("ll")]), text("o hello")]),
    element("p", [text("hello")]),
  ]});
  const ids = (tree: any) => marks(tree).map(mark => mark.properties["data-search-occurrence"]);
  const tree = fixture();
  const transform = rehypeSearchHighlight({query: "hello", scope: "message-a"});
  transform(tree);
  assert.deepEqual(ids(tree), ["message-a:0", "message-a:0", "message-a:0", "message-a:1", "message-a:2"]);
  const rerendered = fixture();
  transform(rerendered);
  assert.deepEqual(ids(rerendered), ids(tree));
  const other = fixture();
  rehypeSearchHighlight({query: "hello", scope: "message-b"})(other);
  assert.equal(ids(other).some(id => ids(tree).includes(id)), false);
});
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";

test("ReactMarkdown mounts the rehype attacher for formatted text, code and escaped text", () => {
  const html = renderToStaticMarkup(createElement(ReactMarkdown, {
    rehypePlugins: [[rehypeSearchHighlight, {query: "hello"}]],
    children: "He**ll**o [hello](https://example.test/hello) `HELLO`\n\n```txt\nhello <script>\n```",
  }));
  assert.equal((html.match(/class="preview-search-highlight"/g) ?? []).length, 6);
  assert.match(html, /<strong><mark class="preview-search-highlight" data-search-occurrence="search:0">ll<\/mark><\/strong>/);
  assert.match(html, /href="https:\/\/example.test\/hello"/);
  assert.match(html, /<code><mark class="preview-search-highlight" data-search-occurrence="search:2">HELLO<\/mark><\/code>/);
  assert.match(html, /&lt;script&gt;/);
  assert.doesNotMatch(html, /<script>/);
});

test("long repeated messages do not overflow function argument limits", () => {
  const tree = {type:"root", children:[element("p",[text("a ".repeat(70000))])]};
  rehypeSearchHighlight({query:"a"})(tree);
  assert.equal(marks(tree).length,70000);
  assert.equal(contents(tree), "a ".repeat(70000));
});

import { SearchHighlightContext, SearchText, SearchJson, SearchMarkdown } from "../components/SearchHighlight";

test("separate text and Markdown components assign independent occurrence scopes", () => {
  const html = renderToStaticMarkup(createElement(SearchHighlightContext.Provider, {value: "hello"},
    createElement(SearchText, {children: "hello hello"}),
    createElement(SearchText, {children: "hello"}),
    createElement(SearchMarkdown, {children: "he**ll**o hello"})));
  const ids = [...html.matchAll(/data-search-occurrence="([^"]+)"/g)].map(match => match[1]);
  assert.equal(ids.length, 7);
  assert.equal(new Set(ids).size, 5);
  assert.equal(ids[3], ids[4]);
  assert.equal(ids[4], ids[5]);
  assert.notEqual(ids[5], ids[6]);
  assert.ok(ids[0].endsWith(":0"));
  assert.ok(ids[1].endsWith(":6"));
});

test("plain text and expanded JSON escape content and clear decorations with the query", () => {
  const plain = (query: string) => renderToStaticMarkup(createElement(SearchHighlightContext.Provider, {value: query},
    createElement(SearchText, {children: '<script>Needle</script> NEEDLE'})));
  assert.equal((plain("needle").match(/<mark /g) ?? []).length, 2);
  assert.doesNotMatch(plain("needle"), /<script>/);
  assert.equal(plain(""), '&lt;script&gt;Needle&lt;/script&gt; NEEDLE');
  const data = {value: '<script>Needle</script>'};
  const json = renderToStaticMarkup(createElement(SearchHighlightContext.Provider, {value: "needle"}, createElement(SearchJson, {data})));
  assert.match(json, /<mark class="preview-search-highlight" data-search-occurrence="[^"]+">Needle<\/mark>/);
  assert.doesNotMatch(json, /<script>/);
  assert.deepEqual(data, {value: '<script>Needle</script>'});
});
