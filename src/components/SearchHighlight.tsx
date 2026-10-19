import { createContext, useContext, useId, useMemo, type ComponentProps } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { JsonView, defaultStyles } from "react-json-view-lite";
import { findPreviewSearchRanges, rehypeSearchHighlight } from "@/lib/preview-search-highlight";

export const SearchHighlightContext = createContext("");
const remarkPlugins = [remarkGfm];

export function SearchMarkdown({ children }: { children: string }) {
  const query = useContext(SearchHighlightContext);
  const scope = useId();
  const plugins = useMemo<ComponentProps<typeof ReactMarkdown>["rehypePlugins"]>(() => query ? [[rehypeSearchHighlight, { query, scope }]] : [], [query, scope]);
  return <ReactMarkdown remarkPlugins={remarkPlugins} rehypePlugins={plugins}>{children}</ReactMarkdown>;
}

export function SearchText({ children }: { children: string }) {
  const query = useContext(SearchHighlightContext);
  const scope = useId();
  const ranges = useMemo(() => findPreviewSearchRanges(children, query), [children, query]);
  let end = 0;
  return <>{ranges.map(range => {
    const before = children.slice(end, range.start);
    end = range.end;
    return <span key={range.start}>{before}<mark className="preview-search-highlight" data-search-occurrence={`${scope}:${range.start}`}>{children.slice(range.start, range.end)}</mark></span>;
  })}{children.slice(end)}</>;
}

export function SearchJson({ data }: { data: object }) {
  const query = useContext(SearchHighlightContext);
  return query
    ? <pre className="whitespace-pre-wrap break-words"><SearchText>{JSON.stringify(data, null, 2)}</SearchText></pre>
    : <JsonView data={data} style={defaultStyles} shouldExpandNode={level => level < 2} />;
}
