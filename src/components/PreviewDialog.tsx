import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Bot,
  Check,
  ChevronDown,
  ChevronsDown,
  FileJson,
  GitBranch,
  Loader2,
  MessageSquare,
  MousePointer2,
  Network,
  Pencil,
  Sparkles,
  Terminal,
  Trash2,
  User,
  Wrench,
  X,
} from "lucide-react";
import { SearchHighlightContext, SearchMarkdown, SearchText, SearchJson } from "@/components/SearchHighlight";
import "react-json-view-lite/dist/index.css";
import { usePreviewOccurrences } from "@/components/usePreviewOccurrences";
import { collectPreviewOccurrences } from "@/lib/preview-search-occurrences";
import { SearchDisclosureContext, useSearchDisclosure, type DisclosureMemory } from "@/components/SearchDisclosure";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { LocalImageAttachments } from "@/components/LocalImageAttachments";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { PreviewEditHistoryDialog } from "@/components/PreviewEditHistoryDialog";
import { PreviewMutationDialogs } from "@/components/PreviewMutationDialogs";
import {
  api,
  type DeletePlan,
  type EditHistory,
  type PreviewEvent,
  type SessionSummary,
  type Settings,
  type UserPromptBrief,
} from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { absoluteTime, formatTimeString, humanTokens } from "@/lib/format";
import { shouldIgnoreTextEditingHotkey } from "@/lib/keyboard";
import { parseUserMessageAttachments } from "@/lib/messageAttachments";
import { PromptTimeline } from "@/components/PromptTimeline";
import { PreviewToolbarActions } from "@/components/PreviewToolbarActions";
import {
  collectRelatedSubagents,
  type RelatedSubagentSession,
} from "@/lib/sessionSource";
import { parseEmbeddedTranscriptPrompt, type EmbeddedTranscriptPrompt } from "@/lib/sessionText";
import {
  buildConversationPreviewRows,
  isProcessGroupExpanded,
  isVisibleConversationEvent,
  summarizeProcessGroupExpansion,
  toConversationDisplayEvent,
  type ConversationPreviewRow,
} from "@/lib/conversationDisplay";
import {
  canDeleteEvent,
  canEditEventText,
  editableText,
  eventMessageLabel,
  extractPreviewEventText as extractText,
  isConversationMessage,
  isEventMessage,
  isStableForkNode,
  parseDiffCommentPrompt,
  payloadType,
  previewEventSearchText,
  rawType,
  subagentEventLabel,
  subagentEventTime,
  type DiffCommentPrompt,
} from "@/lib/previewEvent";
import { cn } from "@/lib/utils";
import { previewWindowStart, findPreviewRowIndex, boundPreviewProcessRows } from "@/lib/previewWindow";
import { useSettings } from "@/stores/settings";
import { toast } from "sonner";
import { previewReadFailure, previewEmptyMessage, type PreviewReadFailure } from "@/lib/preview-read-state";

import { matchesReadingPosition, readingEventSignature, readingPositionKey, readPositions, saveReadingPosition, type ReadingPosition } from "@/lib/preview-reading-position";

import { contextPageRequest, previewTailReached } from "@/lib/preview-context-page";
import { previewSearchContext } from "@/lib/preview-search-context";
import { selectPreviewSearchHit, stepPreviewSearchHit, previewSearchScope } from "@/lib/preview-search-navigation";

type Props = {
  readOnly?: boolean;
  returnLabel?: string;
  onCloseAutoFocus?: () => void;
  open: boolean;
  onOpenChange: (v: boolean) => void;
  session: SessionSummary | null;
  allSessions?: readonly SessionSummary[];
  customRolloutPath?: string;
  codexDir?: string;
  backupDir?: string;
  onForked?: () => void | Promise<void>;
  onEdited?: () => void | Promise<void>;
  initialJump?: PreviewJump | null;
};

export type PreviewJump = {
  eventIndex: number;
  eventOffset: number;
  query: string;
};

type ForkAction = {
  enabled: boolean;
  pending: boolean;
  onSelect: (event: PreviewEvent) => void;
};

type EditActions = {
  enabled: boolean;
  pending: boolean;
  canEditText: (event: PreviewEvent) => boolean;
  canDelete: (event: PreviewEvent) => boolean;
  onEdit: (event: PreviewEvent) => void;
  onDelete: (event: PreviewEvent) => void;
};

type NodeActionSet = {
  fork: ForkAction;
  edit: EditActions;
};

const PAGE = 200;

export function PreviewDialog({
  readOnly = false,
  returnLabel,
  onCloseAutoFocus,
  open,
  onOpenChange,
  session,
  allSessions = [],
  customRolloutPath,
  codexDir,
  backupDir,
  onForked,
  onEdited,
  initialJump,
}: Props) {
  readOnly = readOnly || ["qoder", "workbuddy", "grok", "pi", "dsh", "hermes", "zcode", "qwen", "cline", "copilot", "antigravity"].includes(session?.provider ?? "");
  const rolloutPath = customRolloutPath ?? session?.rollout_path ?? "";
  const provider = session?.provider ?? "codex";
  const manualDisclosures = useMemo(() => new Map<string, boolean>(), [open, provider, rolloutPath]);
  const positionKey = customRolloutPath ? null : readingPositionKey(provider, rolloutPath);
  const restoreRef = useRef<ReadingPosition | null>(null);
  const [readingNotice, setReadingNotice] = useState("");
  const captureReadingRef = useRef<() => void>(() => {});
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [readFailure, setReadFailure] = useState<PreviewReadFailure | null>(null);
  const [events, setEvents] = useState<PreviewEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [done, setDone] = useState(false);
  const [filter, setFilter] = useState("");
  const [searchHit, setSearchHit] = useState<{ query: string; index: number } | null>(null);
  const [contextExpansion, setContextExpansion] = useState({ key: "", before: 0, after: 0 });
  const [contextRead, setContextRead] = useState<{ key: string; direction: "before" | "after"; status: "loading" | "failed" | "loaded" } | null>(null);
  const contextRequestRef = useRef(0);
  const [onlyMsg, setOnlyMsg] = useState(true);
  const [processDefaultCollapsed, setProcessDefaultCollapsed] = useState(true);
  const [processExpansionOverrides, setProcessExpansionOverrides] = useState<
    Record<number, boolean>
  >({});
  const [forkTarget, setForkTarget] = useState<PreviewEvent | null>(null);
  const [forking, setForking] = useState(false);
  const [editTarget, setEditTarget] = useState<PreviewEvent | null>(null);
  const [editText, setEditText] = useState("");
  const [mutating, setMutating] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<PreviewEvent | null>(null);
  const [isSelecting, setIsSelecting] = useState(false);
  const [selectionFirstIndex, setSelectionFirstIndex] = useState<number | null>(null);
  const [selectionSecondIndex, setSelectionSecondIndex] = useState<number | null>(null);
  const [deleteSelectedTarget, setDeleteSelectedTarget] = useState<{ start: number; end: number } | null>(null);
  const [deletePlan, setDeletePlan] = useState<DeletePlan | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [editHistory, setEditHistory] = useState<EditHistory | null>(null);
  const [prompts, setPrompts] = useState<UserPromptBrief[] | null>(null);
  const [totalEvents, setTotalEvents] = useState(0);
  const [activeTimelineIndex, setActiveTimelineIndex] = useState<number | null>(null);
  const [loadingAll, setLoadingAll] = useState(false);
  const [newestFirst, setNewestFirst] = useState(false);
  const newestFirstRef = useRef(false);
  const startOffsetRef = useRef(0);
  const generationRef = useRef(0);
  const loadingAllRef = useRef(false);
  const offsetRef = useRef(0);
  const loadingRef = useRef(false);
  const doneRef = useRef(false);
  const tailReachedRef = useRef(false);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const pendingJumpRef = useRef<number | null>(null);
  const scrollSpyRafRef = useRef(0);
  const jumpSettleRafRef = useRef(0);
  const preferenceSaveRef = useRef<Promise<void>>(Promise.resolve());
  const appSettings = useSettings((state) => state.settings);
  const canForkSession = !readOnly && provider === "codex" && !customRolloutPath && !!session && !!codexDir;
  // 备份/导入预览（customRolloutPath）不允许编辑，只能编辑真实会话文件
  const canMutateSession =
    !readOnly && !customRolloutPath && !!session && !!backupDir && !!rolloutPath;
  const relatedSubagents = useMemo(() => {
    if (!session || session.provider !== "codex" || customRolloutPath) return [];
    return collectRelatedSubagents(session.id, allSessions);
  }, [allSessions, customRolloutPath, session]);

  const previewOnlyMessages = appSettings?.preview_only_messages;
  const previewCollapseProcess = appSettings?.preview_collapse_process;

  useEffect(() => {
    if (previewOnlyMessages === undefined) return;
    setOnlyMsg(previewOnlyMessages);
  }, [previewOnlyMessages]);

  useEffect(() => {
    if (previewCollapseProcess === undefined) return;
    setProcessDefaultCollapsed(previewCollapseProcess);
    setProcessExpansionOverrides({});
  }, [previewCollapseProcess]);

  const persistPreviewPreference = useCallback((patch: Partial<Settings>) => {
    preferenceSaveRef.current = preferenceSaveRef.current.then(async () => {
      try {
        await useSettings.getState().save(patch);
      } catch (error) {
        toast.error("保存预览偏好失败", {
          description: String((error as Error)?.message ?? error),
        });
        await useSettings.getState().load().catch(() => undefined);
      }
    });
  }, []);

  const changeOnlyMsg = useCallback(
    (checked: boolean) => {
      contextRequestRef.current += 1;
      setContextRead(null);
      setOnlyMsg(checked);
      persistPreviewPreference({ preview_only_messages: checked });
    },
    [persistPreviewPreference],
  );

  const changeProcessDefaultCollapsed = useCallback(
    (collapsed: boolean) => {
      setProcessDefaultCollapsed(collapsed);
      setProcessExpansionOverrides({});
      persistPreviewPreference({ preview_collapse_process: collapsed });
    },
    [persistPreviewPreference],
  );

  const changeProcessGroupExpanded = useCallback(
    (key: number, expanded: boolean) => {
      setProcessExpansionOverrides((current) => {
        const defaultExpanded = !processDefaultCollapsed;
        if (expanded === defaultExpanded) {
          if (!(key in current)) return current;
          const next = { ...current };
          delete next[key];
          return next;
        }
        return { ...current, [key]: expanded };
      });
    },
    [processDefaultCollapsed],
  );

  const readWindow = useCallback(async (start: number, count: number, replace: boolean) => {
    const generation = generationRef.current;
    loadingRef.current = true;
    setLoading(true);
    try {
      let next = await api.previewRange(provider, rolloutPath, start, count);
      if (generation !== generationRef.current) return false;
      const remembered = restoreRef.current;
      if (remembered && replace) {
        restoreRef.current = null;
        if (matchesReadingPosition(next, start, remembered)) {
          pendingJumpRef.current = remembered.eventIndex;
          setReadingNotice("已恢复上次阅读位置");
        } else {
          start = 0;
          newestFirstRef.current = false;
          setNewestFirst(false);
          next = await api.previewRange(provider, rolloutPath, 0, count);
          if (generation !== generationRef.current) return false;
          setReadingNotice("原阅读位置已变化，已从最早位置打开");
        }
      }
      if (generation !== generationRef.current) return false;
      if (!replace && start < startOffsetRef.current && next.length !== count) {
        throw new Error("会话在读取期间发生变化，请重新打开预览");
      }
      tailReachedRef.current = previewTailReached(tailReachedRef.current, replace, start < startOffsetRef.current, next.length, count);
      if (replace) {
        setSelectionFirstIndex(null);
        setSelectionSecondIndex(null);
        setProcessExpansionOverrides({});
        startOffsetRef.current = start;
        offsetRef.current = start + next.length;
        setEvents(next);
      } else if (start < startOffsetRef.current) {
        startOffsetRef.current = start;
        setEvents((previous) => [...next, ...previous]);
      } else {
        offsetRef.current = start + next.length;
        setEvents((previous) => [...previous, ...next]);
      }
      setReadFailure(null);
      const finished = newestFirstRef.current ? startOffsetRef.current === 0 : tailReachedRef.current;
      doneRef.current = finished;
      setDone(finished);
      return true;
    } catch (error) {
      if (generation === generationRef.current) {
        restoreRef.current = null;
        pendingJumpRef.current = null;
        doneRef.current = true;
        setDone(true);
        setReadFailure(previewReadFailure(error));
      }
      return false;
    } finally {
      if (generation === generationRef.current) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }, [provider, rolloutPath]);

  const loadMore = useCallback(async () => {
    if (loadingRef.current || loadingAllRef.current || doneRef.current || !rolloutPath) return;
    const start = newestFirstRef.current ? Math.max(0, startOffsetRef.current - PAGE) : offsetRef.current;
    const count = newestFirstRef.current ? startOffsetRef.current - start : PAGE;
    if (count === 0) return;
    await readWindow(start, count, false);
  }, [readWindow, rolloutPath]);

  const waitForIdle = useCallback(async () => {
    const generation = generationRef.current;
    while (loadingRef.current && generation === generationRef.current) {
      await new Promise((resolve) => setTimeout(resolve, 40));
    }
    return generation === generationRef.current;
  }, []);

  // Offset is an ordinal position; event.index remains the original source line.
  const loadUpTo = useCallback(async (targetOffset: number) => {
    if (loadingAllRef.current || !await waitForIdle() || !rolloutPath) return;
    if (targetOffset >= startOffsetRef.current && targetOffset < offsetRef.current) return;
    await readWindow(previewWindowStart(targetOffset, PAGE), PAGE, true);
  }, [readWindow, rolloutPath, waitForIdle]);

  const loadAll = useCallback(async () => {
    if (!await waitForIdle() || !rolloutPath) return;
    setLoadingAll(true);
    loadingAllRef.current = true;
    const generation = generationRef.current;
    try {
      if (!await readWindow(0, PAGE, true)) return;
      while (generation === generationRef.current && offsetRef.current > 0) {
        const previousEnd = offsetRef.current;
        if (!await readWindow(previousEnd, PAGE, false)) break;
        if (offsetRef.current - previousEnd < PAGE) break;
      }
    } finally {
      if (generation === generationRef.current) {
        loadingAllRef.current = false;
        setLoadingAll(false);
      }
    }
  }, [readWindow, rolloutPath, waitForIdle]);

  const navigateEdge = useCallback(async (latest: boolean) => {
    contextRequestRef.current += 1;
    setContextRead(null);
    if (loadingAllRef.current || !await waitForIdle()) return;
    const generation = generationRef.current;
    loadingRef.current = true;
    setLoading(true);
    try {
      const count = latest ? (totalEvents || (await api.previewUserPrompts(provider, rolloutPath)).total_events) : 0;
      if (generation !== generationRef.current) return;
      newestFirstRef.current = latest;
      setNewestFirst(latest);
      pendingJumpRef.current = null;
      setFilter("");
      if (await readWindow(latest ? Math.max(0, count - PAGE) : 0, PAGE, true)
          && generation === generationRef.current) viewportRef.current?.scrollTo({ top: 0 });
    } catch (error) {
      if (generation === generationRef.current) { setReadFailure(previewReadFailure(error)); doneRef.current = true; setDone(true); }
    } finally {
      if (generation === generationRef.current) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }, [provider, readWindow, rolloutPath, totalEvents, waitForIdle]);

  /** 拉取全量用户提问（时间线数据）；属于增强功能，失败时静默降级为无时间线 */
  const loadPrompts = useCallback(async () => {
    const generation = generationRef.current;
    if (!rolloutPath) {
      setPrompts(null);
      setTotalEvents(0);
      return;
    }
    try {
      const list = await api.previewUserPrompts(provider, rolloutPath);
      if (generation !== generationRef.current) return;
      setPrompts(list.prompts);
      setTotalEvents(list.total_events);
    } catch {
      if (generation !== generationRef.current) return;
      setPrompts(null);
      setTotalEvents(0);
    }
  }, [provider, rolloutPath]);

  const resetAndReload = useCallback(() => {
    generationRef.current += 1;
    contextRequestRef.current += 1;
    setContextRead(null);
    tailReachedRef.current = false;
    setReadingNotice("");
    setSearchHit(null);
    setContextExpansion({ key: "", before: 0, after: 0 });
    startOffsetRef.current = 0;
    newestFirstRef.current = false;
    setNewestFirst(false);
    setLoadingAll(false);
    loadingAllRef.current = false;
    setEvents([]);
    setReadFailure(null);
    setProcessExpansionOverrides({});
    setDone(false);
    doneRef.current = false;
    loadingRef.current = false;
    offsetRef.current = 0;
    pendingJumpRef.current = null;
    setActiveTimelineIndex(null);
    setPrompts(null);
    setTotalEvents(0);
    const remembered = restoreRef.current;
    if (remembered) {
      newestFirstRef.current = remembered.newestFirst;
      setNewestFirst(remembered.newestFirst);
      setOnlyMsg(remembered.onlyMessages);
      void readWindow(previewWindowStart(remembered.eventOffset, PAGE), PAGE, true);
    } else {
      void loadMore();
    }
    void loadPrompts();
  }, [loadMore, loadPrompts, readWindow]);

  useEffect(() => {
    if (!open) return;
    if (!rolloutPath) { setReadFailure({ kind: "unavailable", detail: "会话缺少原生文件路径，请返回列表刷新来源。" }); return; }
    setFilter("");
    setIsSelecting(false);
    setSelectionFirstIndex(null);
    setSelectionSecondIndex(null);
    setDeleteSelectedTarget(null);
    setDeletePlan(null);
    restoreRef.current = null;
    if (positionKey && !initialJump) {
      try { restoreRef.current = readPositions(window.localStorage).find((p) => p.key === positionKey) ?? null; } catch { /* Storage may be disabled. */ }
    }
    resetAndReload();
    return () => {
      captureReadingRef.current();
      if (saveTimerRef.current !== null) clearTimeout(saveTimerRef.current);
      generationRef.current += 1;
    };
  }, [open, rolloutPath, positionKey, initialJump, resetAndReload]);

  const timelineIndexSet = useMemo(
    () => (prompts === null ? null : new Set(prompts.map((prompt) => prompt.index))),
    [prompts],
  );

  const deferredFilter = useDeferredValue(filter);
  const normalizedFilter = deferredFilter.trim().toLowerCase();
  const disclosureMemory = useMemo<DisclosureMemory>(() => ({ manual: manualDisclosures, search: new Map() }), [manualDisclosures, normalizedFilter]);
  const searchableEvents = useMemo(
    () => events.map((event) => ({ event, searchText: previewEventSearchText(event) })),
    [events],
  );

  const eligibleEvents = useMemo(() => {
    return searchableEvents.filter(({ event }) => {
      if (
        onlyMsg &&
        (!isConversationMessage(event)
          || !isVisibleConversationEvent(
            event,
            timelineIndexSet,
            initialJump?.eventIndex ?? null,
          ))
      ) {
        return false;
      }
      return true;
    });
  }, [initialJump?.eventIndex, onlyMsg, searchableEvents, timelineIndexSet]);

  const matchingEvents = useMemo(() => eligibleEvents.filter(({ searchText }) =>
    !normalizedFilter || searchText.includes(normalizedFilter)).map(({ event }) => event), [eligibleEvents, normalizedFilter]);
  const searchHitIndices = useMemo(() => {
    const indices = normalizedFilter ? matchingEvents.map(event => event.index) : [];
    return newestFirst ? indices.reverse() : indices;
  }, [matchingEvents, newestFirst, normalizedFilter]);
  const activeSearchHit = selectPreviewSearchHit(searchHitIndices,
    searchHit?.query === normalizedFilter ? searchHit.index : null,
    initialJump?.query.trim().toLowerCase() === normalizedFilter ? initialJump.eventIndex : null);
  const contextKey = JSON.stringify([open, rolloutPath, normalizedFilter, activeSearchHit, onlyMsg]);
  const context = useMemo(() => previewSearchContext(eligibleEvents.map(({ event }) => event.index), activeSearchHit,
    contextExpansion.key === contextKey ? contextExpansion.before : 0,
    contextExpansion.key === contextKey ? contextExpansion.after : 0), [eligibleEvents, activeSearchHit, contextExpansion, contextKey]);
  const hitSet = useMemo(() => new Set(searchHitIndices), [searchHitIndices]);
  const filtered = useMemo(() => {
    if (!normalizedFilter) return matchingEvents;
    const visible = new Set([...searchHitIndices, ...context.indices]);
    return eligibleEvents.filter(({ event }) => visible.has(event.index)).map(({ event }) => event);
  }, [eligibleEvents, matchingEvents, normalizedFilter, searchHitIndices, context]);

  useEffect(() => {
    if (!open || loading || done || normalizedFilter || pendingJumpRef.current !== null) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    // 普通浏览时补满视口；过滤和定位只展示当前窗口，避免隐式读取整段会话。
    if (viewport.scrollHeight <= viewport.clientHeight + 20) {
      void loadMore();
    }
  }, [
    done,
    events.length,
    filtered.length,
    loadMore,
    loading,
    normalizedFilter,
    onlyMsg,
    open,
    processDefaultCollapsed,
    processExpansionOverrides,
  ]);

  /**
   * 有 phase 时仅 final_answer 作为最终答复，commentary 折叠为过程。
   * 无 phase 时使用每轮最后一条 assistant 消息。搜索结果不折叠。
   */
  const chronologicalRows = useMemo<ConversationPreviewRow[]>(() => {
    const displayEvents = onlyMsg ? filtered.map(toConversationDisplayEvent) : filtered;
    if (!onlyMsg || normalizedFilter) {
      return displayEvents.map((event) => ({ type: "event", event }));
    }
    return boundPreviewProcessRows(buildConversationPreviewRows(displayEvents));
  }, [filtered, normalizedFilter, onlyMsg]);

  const rows = useMemo(() => newestFirst ? [...chronologicalRows].reverse() : chronologicalRows, [chronologicalRows, newestFirst]);
  const searchUpdating = filter.trim().toLowerCase() !== normalizedFilter;
  const occurrences = usePreviewOccurrences(viewportRef, activeSearchHit, normalizedFilter, open);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => viewportRef.current,
    estimateSize: () => 150,
    getItemKey: (index) => {
      const row = rows[index];
      return row.type === "event" ? "event-" + row.event.index : "process-" + row.key;
    },
    gap: 16,
    overscan: 6,
    scrollMargin: 24,
  });
  const virtualRows = virtualizer.getVirtualItems();

  const processRowKeys = useMemo(
    () => rows.flatMap((row) => (row.type === "process" ? [row.key] : [])),
    [rows],
  );
  const processExpansionState = useMemo(
    () =>
      summarizeProcessGroupExpansion(
        processRowKeys,
        processDefaultCollapsed,
        processExpansionOverrides,
      ),
    [processDefaultCollapsed, processExpansionOverrides, processRowKeys],
  );

  /** 把待跳转的目标消息滚动到视口顶部并闪烁高亮 */
  const scrollPendingIntoView = useCallback(() => {
    const target = pendingJumpRef.current;
    if (target === null || jumpSettleRafRef.current) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    const rowIndex = findPreviewRowIndex(rows, target);
    const el = viewport.querySelector<HTMLElement>(`[data-event-index="${target}"]`)
      ?? (rowIndex >= 0 ? viewport.querySelector<HTMLElement>(`[data-index="${rowIndex}"]`) : null);
    if (!el) {
      const rowIndex = findPreviewRowIndex(rows, target);
      if (rowIndex >= 0) virtualizer.scrollToIndex(rowIndex, { align: "start" });
      return;
    }
    const generation = generationRef.current;
    // Virtual rows above the target can acquire their real heights after the first
    // scroll. Settle across frames before allowing the restored position to save.
    const align = (remaining: number) => {
      jumpSettleRafRef.current = 0;
      if (generation !== generationRef.current || pendingJumpRef.current !== target || !el.isConnected) return;
      const viewportRect = viewport.getBoundingClientRect();
      const keyword = normalizedFilter ? collectPreviewOccurrences(el)[occurrences.index]?.[0] ?? null : null;
      // Embedded transcripts have their own scroll viewport. Reveal the word there
      // before aligning the outer virtual list, otherwise it can remain clipped.
      if (keyword) {
        for (let parent = keyword.parentElement; parent && parent !== viewport; parent = parent.parentElement) {
          const style = window.getComputedStyle(parent);
          const rect = parent.getBoundingClientRect();
          const word = keyword.getBoundingClientRect();
          if (/auto|scroll/.test(style.overflowY) && parent.scrollHeight > parent.clientHeight) {
            parent.scrollTop += word.top - rect.top - 8;
          }
          if (/auto|scroll/.test(style.overflowX) && parent.scrollWidth > parent.clientWidth) {
            parent.scrollLeft += word.left - rect.left - 8;
          }
        }
      }
      const elRect = (keyword ?? el).getBoundingClientRect();
      viewport.scrollTo({ top: viewport.scrollTop + (elRect.top - viewportRect.top) - 16 });
      if (remaining > 0) jumpSettleRafRef.current = requestAnimationFrame(() => align(remaining - 1));
      else pendingJumpRef.current = null;
    };
    align(3);
    el.classList.remove("preview-jump-flash");
    // 强制 reflow 以便重复跳转同一条时也能重新触发动画
    void el.offsetWidth;
    el.classList.add("preview-jump-flash");
    window.setTimeout(() => el.classList.remove("preview-jump-flash"), 1700);
  }, [rows, virtualizer, normalizedFilter, occurrences.index]);

  useEffect(() => {
    if (!open || !rolloutPath || !initialJump) return;
    setFilter(initialJump.query);
    pendingJumpRef.current = initialJump.eventIndex;
    void loadUpTo(initialJump.eventOffset);
  }, [initialJump, loadUpTo, open, rolloutPath]);

  const scrollSearchRef = useRef(scrollPendingIntoView);
  useEffect(() => { scrollSearchRef.current = scrollPendingIntoView; }, [scrollPendingIntoView]);
  useEffect(() => {
    if (!open || loading || loadingAll || searchUpdating || !normalizedFilter || activeSearchHit === null) return;
    if (jumpSettleRafRef.current) cancelAnimationFrame(jumpSettleRafRef.current);
    jumpSettleRafRef.current = 0;
    pendingJumpRef.current = activeSearchHit;
    setSearchHit({ query: normalizedFilter, index: activeSearchHit });
    scrollSearchRef.current();
  }, [activeSearchHit, normalizedFilter, occurrences.index, open, loading, loadingAll, searchUpdating]);

  const navigateSearchHit = (direction: 1 | -1) => {
    if (loading || loadingAll || searchUpdating) return;
    const index = stepPreviewSearchHit(searchHitIndices, activeSearchHit, direction);
    if (index !== null) {
      contextRequestRef.current += 1;
      setContextRead(null);
      setContextExpansion({ key: "", before: 0, after: 0 });
      setSearchHit({ query: normalizedFilter, index });
    }
  };

  const returnToSearchHit = () => {
    if (activeSearchHit === null) return;
    if (jumpSettleRafRef.current) cancelAnimationFrame(jumpSettleRafRef.current);
    jumpSettleRafRef.current = 0;
    pendingJumpRef.current = activeSearchHit;
    scrollPendingIntoView();
  };

  const contextBeforePage = contextPageRequest("before", context.remainingBefore, startOffsetRef.current, offsetRef.current, tailReachedRef.current, PAGE);
  const contextAfterPage = contextPageRequest("after", context.remainingAfter, startOffsetRef.current, offsetRef.current, tailReachedRef.current, PAGE);
  const currentContextRead = contextRead?.key === contextKey ? contextRead : null;

  const changeContext = async (direction: "before" | "after" | "collapse") => {
    if (activeSearchHit === null || loadingRef.current || loadingAllRef.current || searchUpdating) return;
    const request = ++contextRequestRef.current;
    const generation = generationRef.current;
    if (jumpSettleRafRef.current) cancelAnimationFrame(jumpSettleRafRef.current);
    jumpSettleRafRef.current = 0;
    // Fix the identity before newly loaded matches can change the default selection.
    setSearchHit({ query: normalizedFilter, index: activeSearchHit });
    const page = direction === "collapse" ? null : direction === "before" ? contextBeforePage : contextAfterPage;
    setContextRead(null);
    if (page && direction !== "collapse") {
      setContextRead({ key: contextKey, direction, status: "loading" });
      const succeeded = await readWindow(page.start, page.count, false);
      if (generation !== generationRef.current || request !== contextRequestRef.current) return;
      setContextRead({ key: contextKey, direction, status: succeeded ? "loaded" : "failed" });
      if (!succeeded) return;
    }
    pendingJumpRef.current = activeSearchHit;
    setContextExpansion({ key: contextKey,
      before: direction === "collapse" ? 0 : context.before + (direction === "before" ? 3 : 0),
      after: direction === "collapse" ? 0 : context.after + (direction === "after" ? 3 : 0),
    });
  };

  const changeSearchQuery = (query: string) => {
    contextRequestRef.current += 1;
    setContextRead(null);
    setContextExpansion({ key: "", before: 0, after: 0 });
    if (jumpSettleRafRef.current) cancelAnimationFrame(jumpSettleRafRef.current);
    jumpSettleRafRef.current = 0;
    pendingJumpRef.current = null;
    setSearchHit(null);
    setFilter(query);
  };

  /** 滚动跟随：视口上沿 1/3 处上方最近的一条用户提问视为当前时间线位置。 */
  const updateActiveFromScroll = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const anchors = viewport.querySelectorAll<HTMLElement>("[data-timeline-anchor]");
    if (anchors.length === 0) return;
    const threshold =
      viewport.getBoundingClientRect().top + viewport.clientHeight * 0.33;
    let current: number | null = null;
    for (const node of anchors) {
      if (node.getBoundingClientRect().top > threshold) break;
      current = Number(node.dataset.eventIndex);
    }
    if (current === null) current = Number(anchors[0].dataset.eventIndex);
    if (Number.isFinite(current)) setActiveTimelineIndex(current);
  }, []);

  const jumpToTimelineMessage = useCallback(
    (prompt: UserPromptBrief) => {
      contextRequestRef.current += 1;
      setContextRead(null);
      setContextExpansion({ key: "", before: 0, after: 0 });
      setActiveTimelineIndex(prompt.index);
      pendingJumpRef.current = prompt.index;
      // 文本过滤可能把目标消息隐藏，跳转时清空
      setFilter("");
      void loadUpTo(prompt.offset).then(() => scrollPendingIntoView());
    },
    [loadUpTo, scrollPendingIntoView],
  );

  // 加载/过滤变化后：完成待跳转的定位。搜索期间无需扫描整段 DOM 更新时间线。
  useEffect(() => {
    scrollPendingIntoView();
    if (normalizedFilter) return;
    if (scrollSpyRafRef.current) cancelAnimationFrame(scrollSpyRafRef.current);
    scrollSpyRafRef.current = requestAnimationFrame(updateActiveFromScroll);
  }, [filtered, normalizedFilter, scrollPendingIntoView, updateActiveFromScroll, virtualRows]);

  useEffect(() => {
    return () => {
      if (scrollSpyRafRef.current) cancelAnimationFrame(scrollSpyRafRef.current);
      if (jumpSettleRafRef.current) cancelAnimationFrame(jumpSettleRafRef.current);
      jumpSettleRafRef.current = 0;
    };
  }, []);

  // Read from the current render at flush time; never retain transcript text in storage.
  // Do not replace a good bookmark with an error, search result, or pending jump.
  const captureReading = useCallback(() => {
    if (!positionKey || !rolloutPath || loadingRef.current || readFailure || normalizedFilter
        || initialJump || restoreRef.current || pendingJumpRef.current !== null) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    const bounds = viewport.getBoundingClientRect();
    const visible = Array.from(viewport.querySelectorAll<HTMLElement>("[data-index]"))
      .find((node) => { const rect = node.getBoundingClientRect(); return rect.bottom > bounds.top + 16 && rect.top < bounds.bottom; });
    if (!visible) return;
    const row = rows[Number(visible.dataset.index)];
    if (!row) return;
    const event = row.type === "event" ? row.event : row.events[0];
    const ordinal = events.findIndex((item) => item.index === event.index);
    if (ordinal < 0) return;
    try {
      saveReadingPosition(window.localStorage, { key: positionKey, eventIndex: event.index,
        eventOffset: startOffsetRef.current + ordinal, signature: readingEventSignature(events[ordinal]),
        newestFirst: newestFirstRef.current, onlyMessages: onlyMsg, savedAt: Date.now() });
    } catch { /* Reading remains available when storage is disabled. */ }
  }, [events, initialJump, normalizedFilter, onlyMsg, positionKey, readFailure, rolloutPath, rows]);
  useEffect(() => { captureReadingRef.current = captureReading; }, [captureReading]);
  useEffect(() => {
    if (!open) return;
    const flush = () => captureReadingRef.current();
    const hide = () => { if (document.visibilityState === "hidden") flush(); };
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", hide);
    return () => { window.removeEventListener("pagehide", flush); document.removeEventListener("visibilitychange", hide); };
  }, [open]);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    if (saveTimerRef.current !== null) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => captureReadingRef.current(), 400);
    const el = e.currentTarget;
    if (!loadingAll && !normalizedFilter && pendingJumpRef.current === null && el.scrollHeight - el.scrollTop - el.clientHeight < 200) {
      void loadMore();
    }
    if (scrollSpyRafRef.current) cancelAnimationFrame(scrollSpyRafRef.current);
    scrollSpyRafRef.current = requestAnimationFrame(updateActiveFromScroll);
  };

  const onPreviewKeyDown = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if (shouldIgnoreTextEditingHotkey(e.target)) return;

      const viewport = viewportRef.current;
      if (!viewport) return;

      const maxScrollTop = Math.max(viewport.scrollHeight - viewport.clientHeight, 0);
      const pageDelta = Math.max(Math.floor(viewport.clientHeight * 0.9), 120);
      let nextScrollTop: number | null = null;
      let keepAtBottomAfterLoad = false;

      switch (e.key) {
        case "Home":
          nextScrollTop = 0;
          break;
        case "End":
          nextScrollTop = maxScrollTop;
          keepAtBottomAfterLoad = true;
          break;
        case "PageUp":
          nextScrollTop = viewport.scrollTop - pageDelta;
          break;
        case "PageDown":
          nextScrollTop = viewport.scrollTop + pageDelta;
          break;
        default:
          return;
      }

      e.preventDefault();

      const clampedScrollTop = Math.max(0, Math.min(nextScrollTop, maxScrollTop));
      viewport.scrollTo({ top: clampedScrollTop });

      if (keepAtBottomAfterLoad) {
        void loadMore().then(() => {
          requestAnimationFrame(() => {
            const nextViewport = viewportRef.current;
            if (!nextViewport) return;
            nextViewport.scrollTo({
              top: Math.max(nextViewport.scrollHeight - nextViewport.clientHeight, 0),
            });
          });
        });
        return;
      }

      if (maxScrollTop - clampedScrollTop < 200) {
        void loadMore();
      }
    },
    [loadMore],
  );

  const copyResume = async () => {
    if (!session) return;
    try {
      const text = await api.copyResumeCommand(
        session.provider,
        session.id,
        session.cwd,
        session.resume_command,
      );
      toast.success(`已复制：${text}`);
    } catch (e: any) {
      toast.error("复制失败：" + String(e?.message ?? e));
    }
  };

  const copySessionId = async () => {
    if (!session) return;
    try {
      await copyText(session.id);
      toast.success(`已复制会话 ID：${session.id}`);
    } catch (e: any) {
      toast.error("复制会话 ID 失败：" + String(e?.message ?? e));
    }
  };

  const reveal = async () => {
    if (!session) return;
    try {
      await api.revealCwd(session.cwd);
    } catch (e: any) {
      toast.error("打开失败：" + String(e?.message ?? e));
    }
  };

  const copyPath = async () => {
    if (!rolloutPath) return;
    try {
      await copyText(rolloutPath);
      toast.success("已复制 rollout 路径");
    } catch (e: any) {
      toast.error("复制 rollout 路径失败：" + String(e?.message ?? e));
    }
  };

  const requestForkAt = (event: PreviewEvent) => {
    if (!canForkSession) return;
    setForkTarget(event);
  };

  const confirmForkAt = async () => {
    if (!session || !codexDir || !rolloutPath || !forkTarget) return;
    setForking(true);
    try {
      const report = await api.forkSessionAtEvent({
        codex_dir: codexDir,
        session_id: session.id,
        rollout_path: rolloutPath,
        event_index: forkTarget.index,
      });
      toast.success("已创建回溯分支", {
        description: `新会话 ${report.new_id.slice(0, 8)}，已复制 ${report.included_lines} 行`,
      });
      setForkTarget(null);
      onOpenChange(false);
      await onForked?.();
    } catch (e: any) {
      toast.error("创建回溯分支失败", {
        description: String(e?.message ?? e),
      });
    } finally {
      setForking(false);
    }
  };

  const requestEditAt = (event: PreviewEvent) => {
    if (!canMutateSession) return;
    setEditText(editableText(event));
    setEditTarget(event);
  };

  const confirmEdit = async () => {
    if (!session || !backupDir || !rolloutPath || !editTarget) return;
    setMutating(true);
    try {
      const report = await api.editSessionEventText({
        provider,
        rollout_path: rolloutPath,
        session_id: session.id,
        backup_dir: backupDir,
        line_no: editTarget.index,
        new_text: editText,
      });
      toast.success(
        provider === "opencode"
          ? `已改写 OpenCode 消息（${report.changed_lines} 个内容块）`
          : `已改写消息（含镜像共 ${report.changed_lines} 行）`,
        {
          description: report.snapshot_created
            ? `编辑前已自动保存原始快照 ${report.snapshot_created}`
            : "本次编辑已记入编辑历史，可随时撤销",
        },
      );
      setEditTarget(null);
      resetAndReload();
      await onEdited?.();
    } catch (e: any) {
      toast.error("改写失败", { description: String(e?.message ?? e) });
    } finally {
      setMutating(false);
    }
  };

  const requestDeleteAt = (event: PreviewEvent) => {
    if (!canMutateSession || !rolloutPath) return;
    setDeletePlan(null);
    setDeleteTarget(event);
    api
      .planSessionEventDeletion(provider, rolloutPath, [event.index])
      .then(setDeletePlan)
      .catch((e: any) => {
        toast.error("生成删除计划失败", { description: String(e?.message ?? e) });
        setDeleteTarget(null);
      });
  };

  const confirmDelete = async () => {
    if (!session || !backupDir || !rolloutPath || !deleteTarget) return;
    setMutating(true);
    try {
      const report = await api.deleteSessionEvents({
        provider,
        rollout_path: rolloutPath,
        session_id: session.id,
        backup_dir: backupDir,
        line_nos: [deleteTarget.index],
      });
      toast.success(`已删除 ${report.deleted_lines} 个事件`, {
        description: report.snapshot_created
          ? `删除前已自动保存原始快照 ${report.snapshot_created}`
          : "本次删除已记入编辑历史，可随时撤销",
      });
      setDeleteTarget(null);
      setDeletePlan(null);
      resetAndReload();
      await onEdited?.();
    } catch (e: any) {
      toast.error("删除失败", { description: String(e?.message ?? e) });
    } finally {
      setMutating(false);
    }
  };

  const requestDeleteSelected = () => {
    if (!canMutateSession || !rolloutPath || selectionFirstIndex === null || selectionSecondIndex === null) return;
    const start = Math.min(selectionFirstIndex, selectionSecondIndex);
    const end = Math.max(selectionFirstIndex, selectionSecondIndex);
    setDeletePlan(null);
    setDeleteSelectedTarget({ start, end });
    const indices = events
      .filter((e) => e.index >= start && e.index <= end && canDeleteEvent(provider, e))
      .map((e) => e.index);
    api
      .planSessionEventDeletion(provider, rolloutPath, indices)
      .then(setDeletePlan)
      .catch((e: any) => {
        toast.error("生成删除计划失败", { description: String(e?.message ?? e) });
        setDeleteSelectedTarget(null);
      });
  };

  const confirmDeleteSelected = async () => {
    if (!session || !backupDir || !rolloutPath || !deleteSelectedTarget) return;
    setMutating(true);
    try {
      const { start, end } = deleteSelectedTarget;
      const indices = events
        .filter((e) => e.index >= start && e.index <= end && canDeleteEvent(provider, e))
        .map((e) => e.index);
      const report = await api.deleteSessionEvents({
        provider,
        rollout_path: rolloutPath,
        session_id: session.id,
        backup_dir: backupDir,
        line_nos: indices,
      });
      toast.success(`已删除 ${report.deleted_lines} 个事件`, {
        description: report.snapshot_created
          ? `删除前已自动保存原始快照 ${report.snapshot_created}`
          : "本次删除已记入编辑历史，可随时撤销",
      });
      setDeleteSelectedTarget(null);
      setDeletePlan(null);
      setIsSelecting(false);
      setSelectionFirstIndex(null);
      setSelectionSecondIndex(null);
      resetAndReload();
      await onEdited?.();
    } catch (e: any) {
      toast.error("删除失败", { description: String(e?.message ?? e) });
    } finally {
      setMutating(false);
    }
  };

  const loadEditHistory = useCallback(async () => {
    if (!session || !backupDir || !rolloutPath) return;
    try {
      const h = await api.sessionEditHistory({
        provider,
        rollout_path: rolloutPath,
        session_id: session.id,
        backup_dir: backupDir,
      });
      setEditHistory(h);
    } catch (e: any) {
      toast.error("读取编辑历史失败", { description: String(e?.message ?? e) });
    }
  }, [backupDir, provider, rolloutPath, session]);

  const openEditHistory = () => {
    setEditHistory(null);
    setHistoryOpen(true);
    void loadEditHistory();
  };

  const undoLastEdit = async () => {
    if (!session || !backupDir || !rolloutPath) return;
    setMutating(true);
    try {
      await api.undoLastSessionEdit({
        provider,
        rollout_path: rolloutPath,
        session_id: session.id,
        backup_dir: backupDir,
      });
      toast.success("已撤销最近一次编辑");
      await loadEditHistory();
      resetAndReload();
      await onEdited?.();
    } catch (e: any) {
      toast.error("撤销失败", { description: String(e?.message ?? e) });
    } finally {
      setMutating(false);
    }
  };

  const restoreSnapshot = async (name: string) => {
    if (!session || !backupDir || !rolloutPath) return;
    setMutating(true);
    try {
      const report = await api.restoreSessionEditSnapshot({
        provider,
        rollout_path: rolloutPath,
        session_id: session.id,
        backup_dir: backupDir,
        snapshot_name: name,
      });
      toast.success(`已还原快照 ${name}`, {
        description: report.snapshot_created
          ? `还原前状态已另存为 ${report.snapshot_created}`
          : undefined,
      });
      await loadEditHistory();
      resetAndReload();
      await onEdited?.();
    } catch (e: any) {
      toast.error("还原快照失败", { description: String(e?.message ?? e) });
    } finally {
      setMutating(false);
    }
  };

  const editActions: EditActions = {
    enabled: canMutateSession,
    pending: mutating,
    canEditText: (e) => canEditEventText(provider, e),
    canDelete: (e) => canDeleteEvent(provider, e),
    onEdit: requestEditAt,
    onDelete: requestDeleteAt,
  };

  return (
    <>
    <Dialog open={open} onOpenChange={(next) => { if (!next) captureReadingRef.current(); onOpenChange(next); }}>
      <DialogContent
        className="flex h-[90vh] max-w-[96vw] min-w-0 flex-col gap-0 overflow-hidden p-0 sm:max-w-[1200px]"
        onKeyDown={onPreviewKeyDown}
        onCloseAutoFocus={onCloseAutoFocus ? event => { event.preventDefault(); onCloseAutoFocus(); } : undefined}
      >
        <DialogHeader className="relative min-w-0 border-b border-border/60 px-6 pb-3.5 pt-4 after:pointer-events-none after:absolute after:inset-x-0 after:-bottom-px after:h-px after:bg-gradient-to-r after:from-transparent after:via-border/50 after:to-transparent">
          {returnLabel && <Button size="sm" variant="outline" className="mb-3" onClick={() => { captureReadingRef.current(); onOpenChange(false); }}>{returnLabel}</Button>}
          <div className="flex items-start gap-3.5">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-border/60 bg-gradient-to-br from-muted to-muted/40 shadow-sm">
              <Sparkles className="h-[18px] w-[18px] text-muted-foreground" />
            </div>
            <div className="min-w-0 flex-1">
              <DialogTitle
                className="truncate pr-4 text-[15px] font-semibold tracking-tight"
                title={session?.title || "预览会话"}
              >
                {session?.title || "预览会话"}
              </DialogTitle>
              <DialogDescription className="sr-only">
                查看会话消息、过程事件和对话时间线。
              </DialogDescription>
              {session && (
                <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                  <span className="font-mono text-foreground/70">{session.id.slice(0, 8)}</span>
                  {session.model && (
                    <>
                      <Dot />
                      <Badge variant="secondary" className="h-5 px-1.5 font-normal">
                        {session.model}
                        {session.reasoning_effort ? ` · ${session.reasoning_effort}` : ""}
                      </Badge>
                    </>
                  )}
                  {session.tokens_used > 0 && (
                    <>
                      <Dot />
                      <span className="tabular-nums">
                        {humanTokens(session.tokens_used)} token
                      </span>
                    </>
                  )}
                  {session.cwd_display && (
                    <>
                      <Dot />
                      <span className="min-w-0 truncate" title={session.cwd}>
                        {session.cwd_display}
                      </span>
                    </>
                  )}
                  <Dot />
                  <span className="text-[11px] text-muted-foreground">
                    显示 <span className="tabular-nums text-foreground/80">{filtered.length}</span>
                    <span className="mx-1 text-muted-foreground/50">/</span>
                    已加载 <span className="tabular-nums text-foreground/80">{events.length}</span>
                    {totalEvents > 0 && (
                      <>
                        <span className="mx-1 text-muted-foreground/50">/</span>
                        共 <span className="tabular-nums text-foreground/80">{totalEvents}</span>
                      </>
                    )}{" "}
                    条
                    <span className="ml-1 text-muted-foreground/70">
                      {readFailure ? "· 读取中断" : !done ? (newestFirst ? "· 滚动加载更早" : "· 滚动加载更多") : (newestFirst ? "· 已到最早" : "· 已到末尾")}
                    </span>
                  </span>
                </div>
              )}
            </div>
          </div>

          <div className="mt-3.5 flex flex-wrap items-center gap-2">
            <Input
              placeholder="在事件中过滤…"
              value={filter}
              aria-label="搜索正文消息"
              onChange={(e) => changeSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  e.stopPropagation();
                  navigateSearchHit(e.shiftKey ? -1 : 1);
                }
              }}
              className="h-8 w-64 border-border/70"
            />
            {filter.trim() && <div className="flex flex-wrap items-center gap-1.5 text-xs" aria-label="正文命中导航">
              <span role="status" aria-live="polite" className="tabular-nums">
                {searchUpdating ? "更新命中…" : `${activeSearchHit === null ? 0 : searchHitIndices.indexOf(activeSearchHit) + 1} / ${searchHitIndices.length} 条命中消息`}
              </span>
              <Button variant="outline" size="sm" className="h-8 text-xs" aria-label="上一处命中" title="上一处命中（Shift+Enter），首尾循环"
                disabled={loading || loadingAll || searchUpdating || searchHitIndices.length < 2} onClick={() => navigateSearchHit(-1)}>上条</Button>
              <Button variant="outline" size="sm" className="h-8 text-xs" aria-label="下一处命中" title="下一处命中（Enter），首尾循环"
                disabled={loading || loadingAll || searchUpdating || searchHitIndices.length < 2} onClick={() => navigateSearchHit(1)}>下条</Button>
              <span className="border-l pl-2 tabular-nums" role="status" aria-live="polite" aria-label="本条关键词命中">
                {searchUpdating ? "更新定位…" : `本条 ${occurrences.total ? occurrences.index + 1 : 0} / ${occurrences.total} 处`}
              </span>
              <Button variant="outline" size="sm" className="h-8 text-xs" aria-label="本条上一处" title="当前消息内上一处关键词，首尾循环"
                disabled={loading || loadingAll || searchUpdating || occurrences.total < 2} onClick={() => occurrences.step(-1)}>上一处</Button>
              <Button variant="outline" size="sm" className="h-8 text-xs" aria-label="本条下一处" title="当前消息内下一处关键词，首尾循环"
                disabled={loading || loadingAll || searchUpdating || occurrences.total < 2} onClick={() => occurrences.step(1)}>下一处</Button>
              <div className="flex flex-wrap items-center gap-1.5 border-l pl-2" aria-label="命中上下文">
                <Button variant="outline" size="sm" className="h-8 text-xs" aria-label="展开前 3 条上下文" title="按时间顺序展开更早的消息；不足时最多读取相邻一页"
                  disabled={loading || loadingAll || searchUpdating || activeSearchHit === null || (!context.remainingBefore && !contextBeforePage)} onClick={() => void changeContext("before")}>{!context.remainingBefore && contextBeforePage ? "加载更早一页" : "前 3 条"}</Button>
                <Button variant="outline" size="sm" className="h-8 text-xs" aria-label="展开后 3 条上下文" title="按时间顺序展开更晚的消息；不足时最多读取相邻一页"
                  disabled={loading || loadingAll || searchUpdating || activeSearchHit === null || (!context.remainingAfter && !contextAfterPage)} onClick={() => void changeContext("after")}>{!context.remainingAfter && contextAfterPage ? "加载更晚一页" : "后 3 条"}</Button>
                <span role="status" aria-label="上下文范围" className="text-muted-foreground">前 {context.before} / 后 {context.after} 条 · 已加载范围</span>
                <Button variant="ghost" size="sm" className="h-8 text-xs" disabled={loading || loadingAll || searchUpdating || activeSearchHit === null}
                  onClick={returnToSearchHit}>返回当前命中</Button>
                {(context.before > 0 || context.after > 0) && <Button variant="ghost" size="sm" className="h-8 text-xs"
                  disabled={loading || loadingAll || searchUpdating} onClick={() => void changeContext("collapse")}>收起上下文</Button>}
                {currentContextRead && <span role="status" aria-label="上下文读取状态" className="text-muted-foreground">
                  {currentContextRead.status === "loading" ? "正在读取相邻页…" : currentContextRead.status === "failed" ? "读取失败，已保留当前命中" : "已读取一页；仅展开符合当前筛选的相邻消息"}
                </span>}
                {currentContextRead?.status === "failed" && <Button variant="outline" size="sm" className="h-8 text-xs"
                  disabled={loading || loadingAll || searchUpdating} onClick={() => void changeContext(currentContextRead.direction)}>重试上下文读取</Button>}
              </div>
              <span className="text-muted-foreground" title="本条计数仅包含已展开正文，折叠区域展开后更新；跨格式的同一匹配只计一次">本条按已展开正文定位</span>
              <span className="text-muted-foreground">{previewSearchScope(startOffsetRef.current, offsetRef.current, totalEvents, newestFirst, done, !!readFailure)} · 每条消息计一次</span>
              <Button variant="ghost" size="sm" className="h-8 text-xs" onClick={() => changeSearchQuery("")}>清除搜索</Button>
            </div>}
            <label
              htmlFor="only-msg"
              className="group flex h-8 cursor-pointer items-center gap-2 rounded-md border border-border/70 bg-muted/30 px-2.5 transition-colors hover:bg-muted/50"
            >
              <Switch id="only-msg" checked={onlyMsg} onCheckedChange={changeOnlyMsg} />
              <Label htmlFor="only-msg" className="cursor-pointer text-xs">
                仅看对话消息
              </Label>
            </label>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1.5 border-border/70 bg-muted/30 px-2.5 text-xs font-normal hover:bg-muted/50"
                  disabled={!onlyMsg || Boolean(filter)}
                  title={
                    !onlyMsg
                      ? "仅看对话消息开启后可统一收起或展开过程消息"
                      : filter
                        ? "过滤结果会直接显示命中的消息"
                        : "统一收起或展开当前会话的过程消息"
                  }
                >
                  <Bot className="h-3.5 w-3.5" />
                  过程消息
                  <ChevronDown className="h-3 w-3 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="min-w-[7rem]">
                <DropdownMenuItem onSelect={() => changeProcessDefaultCollapsed(true)}>
                  <span>全部收起</span>
                  {processExpansionState === "collapsed" && (
                    <Check className="h-4 w-4 text-primary" />
                  )}
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => changeProcessDefaultCollapsed(false)}>
                  <span>全部展开</span>
                  {processExpansionState === "expanded" && (
                    <Check className="h-4 w-4 text-primary" />
                  )}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
            <Button variant="outline" size="sm" className="h-8 text-xs" disabled={loading || loadingAll}
              onClick={() => void navigateEdge(!newestFirst)} aria-label="切换对话排序">
              {newestFirst ? "最新在前 ↓" : "最早在前 ↑"}
            </Button>
            <Button variant="ghost" size="sm" className="h-8 text-xs" disabled={loading || loadingAll}
              onClick={() => void navigateEdge(false)}>最早</Button>
            <Button variant="ghost" size="sm" className="h-8 text-xs" disabled={loading || loadingAll}
              onClick={() => void navigateEdge(true)}>最新</Button>
            {events.length > 0 && (startOffsetRef.current > 0 || !done) && (
              <Button
                variant="outline"
                size="sm"
                className="h-8 gap-1.5 border-border/70 bg-muted/30 px-2.5 text-xs font-normal hover:bg-muted/50"
                disabled={loading || loadingAll}
                onClick={() => void loadAll()}
              >
                {loadingAll ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <ChevronsDown className="h-3.5 w-3.5" />
                )}
                {loadingAll ? "加载中…" : "加载全部"}
              </Button>
            )}
            {canMutateSession && !isSelecting && (
              <Button
                variant="outline"
                size="sm"
                className="h-8 gap-1.5 border-border/70 bg-muted/30 px-2.5 text-xs font-normal hover:bg-muted/50"
                onClick={() => {
                  setIsSelecting(true);
                  setSelectionFirstIndex(null);
                  setSelectionSecondIndex(null);
                }}
              >
                <MousePointer2 className="h-3.5 w-3.5" />
                开始选取
              </Button>
            )}
            {canMutateSession && isSelecting && (
              <>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1.5 border-border/70 bg-muted/30 px-2.5 text-xs font-normal hover:bg-muted/50"
                  onClick={() => {
                    setIsSelecting(false);
                    setSelectionFirstIndex(null);
                    setSelectionSecondIndex(null);
                  }}
                >
                  <X className="h-3.5 w-3.5" />
                  结束选取
                </Button>
                {selectionFirstIndex !== null && selectionSecondIndex !== null && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 gap-1.5 border-destructive/50 bg-destructive/10 px-2.5 text-xs font-normal text-destructive hover:bg-destructive/20"
                    onClick={requestDeleteSelected}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    删除选中
                    <span className="tabular-nums">
                      {events.filter(
                        (e) =>
                          e.index >= Math.min(selectionFirstIndex, selectionSecondIndex) &&
                          e.index <= Math.max(selectionFirstIndex, selectionSecondIndex) &&
                          canDeleteEvent(provider, e),
                      ).length}
                      条
                    </span>
                  </Button>
                )}
                {selectionFirstIndex !== null && selectionSecondIndex === null && (
                  <span className="text-xs text-muted-foreground">请点击第二个事件完成选取</span>
                )}
              </>
            )}
            <PreviewToolbarActions
              hasSession={!!session}
              canCopyResume={!["qoder", "workbuddy", "grok", "pi", "dsh", "hermes", "zcode", "qwen", "cline", "copilot", "antigravity"].includes(provider)}
              canOpenEditHistory={canMutateSession}
              onCopySessionId={copySessionId}
              onCopyResume={copyResume}
              onRevealDirectory={reveal}
              onOpenEditHistory={openEditHistory}
              onCopyPath={copyPath}
            />
          </div>
        </DialogHeader>

        {readingNotice && !readFailure && <div role="status" className="flex shrink-0 flex-wrap items-center gap-2 border-b px-6 py-2 text-xs text-muted-foreground">
          <span>{readingNotice}</span>
          <Button variant="ghost" size="sm" disabled={loading || loadingAll} onClick={() => { setReadingNotice(""); void navigateEdge(false); }}>从最早位置阅读</Button>
        </div>}
        {readFailure && <div role="alert" className="shrink-0 space-y-2 border-b px-6 py-3 text-sm">
          <p className="font-medium text-destructive">{readFailure.kind === "unavailable" ? "会话文件已失效" : "正文读取失败"}</p>
          <p className="text-xs text-muted-foreground">{readFailure.kind === "unavailable" ? "文件可能已移动、删除或不再是可读取的会话文件。请检查来源；文件恢复后可重试。" : "可能存在权限、文件内容或连接问题，请查看错误详情后重试。"}{events.length > 0 ? " 下方仅保留此前已读取的部分内容，不代表完整会话。" : " 这不表示会话为空。"}</p>
          <details className="text-xs"><summary className="cursor-pointer">读取错误详情</summary><p className="max-h-24 overflow-auto whitespace-pre-wrap break-all">{readFailure.detail}</p></details>
          <Button size="sm" variant="outline" disabled={loading || loadingAll || !rolloutPath} onClick={resetAndReload}>{loading ? "正在重试…" : "重新读取正文"}</Button>
          <span className="ml-2 text-xs text-muted-foreground">从最早位置重新读取，不修改原文件。</span>
        </div>}
        <div className="relative min-h-0 flex-1">
          <ScrollArea
            className="h-full bg-muted/30"
            viewportRef={viewportRef}
            onViewportScroll={onScroll}
          >
            <div className="mx-auto w-full max-w-3xl min-w-0 space-y-4 overflow-x-hidden px-6 py-6">
              {!onlyMsg && relatedSubagents.length > 0 && (
                <SubagentOverview key={session?.id} items={relatedSubagents} />
              )}

              {!readFailure && filtered.length === 0 && !loading && (onlyMsg || relatedSubagents.length === 0) && (
                <div className="flex flex-col items-center justify-center gap-2 py-16 text-center text-muted-foreground">
                  <Sparkles className="h-8 w-8 opacity-50" />
                  <div className="text-sm">
                    {previewEmptyMessage(events.length, startOffsetRef.current, !!normalizedFilter)}
                  </div>
                </div>
              )}

              {startOffsetRef.current > 0 && !newestFirst && (
                <Button variant="outline" size="sm" disabled={loading || loadingAll}
                  onClick={() => { void readWindow(Math.max(0, startOffsetRef.current - PAGE), PAGE, true); }}>
                  查看前一页
                </Button>
              )}
              <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
              {virtualRows.map((virtualRow) => {
                const row = rows[virtualRow.index];
                return <div key={virtualRow.key} ref={virtualizer.measureElement} data-index={virtualRow.index}
                  aria-current={normalizedFilter && row.type === "event" && row.event.index === activeSearchHit ? "true" : undefined}
                  className={cn("absolute left-0 top-0 w-full min-w-0", normalizedFilter && row.type === "event" && row.event.index === activeSearchHit && "rounded-lg ring-2 ring-primary/60")}
                  style={{ transform: `translateY(${virtualRow.start - virtualizer.options.scrollMargin}px)` }}>
                {row.type === "process" ? (
                  <ProcessTurnGroup
                    key={`process-${row.key}`}
                    events={row.events}
                    expanded={isProcessGroupExpanded(
                      row.key,
                      processDefaultCollapsed,
                      processExpansionOverrides,
                    )}
                    onExpandedChange={(expanded) =>
                      changeProcessGroupExpanded(row.key, expanded)
                    }
                  >
                    {(event) => {
                      const inRange =
                        isSelecting &&
                        selectionFirstIndex !== null &&
                        selectionSecondIndex !== null &&
                        event.index >= Math.min(selectionFirstIndex, selectionSecondIndex) &&
                        event.index <= Math.max(selectionFirstIndex, selectionSecondIndex);
                      const isStart =
                        isSelecting &&
                        selectionFirstIndex !== null &&
                        selectionSecondIndex === null &&
                        event.index === selectionFirstIndex;
                      return (
                      <div
                        key={event.index}
                        data-event-index={event.index}
                        className={cn(
                          isSelecting && "cursor-pointer",
                          inRange && "bg-destructive/10 ring-1 ring-destructive/30",
                          isStart && "bg-primary/10 ring-1 ring-primary/30",
                        )}
                        onClick={
                          isSelecting
                            ? () => {
                                if (selectionFirstIndex === null) {
                                  setSelectionFirstIndex(event.index);
                                } else if (selectionSecondIndex === null) {
                                  setSelectionSecondIndex(event.index);
                                } else {
                                  setSelectionFirstIndex(event.index);
                                  setSelectionSecondIndex(null);
                                }
                              }
                            : undefined
                        }
                      >
                        <EventBubble
                          e={event}
                          query={normalizedFilter}
                          disclosureMemory={disclosureMemory}
                          actions={{
                            fork: {
                              enabled: canForkSession && isStableForkNode(event),
                              pending: forking,
                              onSelect: requestForkAt,
                            },
                            edit: editActions,
                          }}
                        />
                      </div>
                      );
                    }}
                  </ProcessTurnGroup>
                ) : (
                  <div
                    key={row.event.index}
                    data-event-index={row.event.index}
                    data-timeline-anchor={timelineIndexSet?.has(row.event.index) || undefined}
                    className={cn(
                      isSelecting && "cursor-pointer",
                      isSelecting &&
                        selectionFirstIndex !== null &&
                        selectionSecondIndex !== null &&
                        row.event.index >= Math.min(selectionFirstIndex, selectionSecondIndex) &&
                        row.event.index <= Math.max(selectionFirstIndex, selectionSecondIndex) &&
                        "bg-destructive/10 ring-1 ring-destructive/30",
                      isSelecting &&
                        selectionFirstIndex !== null &&
                        selectionSecondIndex === null &&
                        row.event.index === selectionFirstIndex &&
                        "bg-primary/10 ring-1 ring-primary/30",
                    )}
                    onClick={
                      isSelecting
                        ? () => {
                            if (selectionFirstIndex === null) {
                              setSelectionFirstIndex(row.event.index);
                            } else if (selectionSecondIndex === null) {
                              setSelectionSecondIndex(row.event.index);
                            } else {
                              setSelectionFirstIndex(row.event.index);
                              setSelectionSecondIndex(null);
                            }
                          }
                        : undefined
                    }
                  >
                    {normalizedFilter && !hitSet.has(row.event.index) && <div className="mb-1 px-2 text-xs text-muted-foreground" data-search-context="true">上下文 · 非命中消息</div>}
                    <EventBubble
                      e={row.event}
                      query={normalizedFilter}
                      disclosureMemory={disclosureMemory}
                      actions={{
                        fork: {
                          enabled: canForkSession && isStableForkNode(row.event),
                          pending: forking,
                          onSelect: requestForkAt,
                        },
                        edit: editActions,
                      }}
                    />
                  </div>
                )}
                </div>;
              })}
              </div>

              {loading && (
                <div className="flex justify-center py-4 text-xs text-muted-foreground">加载中…</div>
              )}
              {!done && events.length > 0 && (
                <div className="flex justify-center pt-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8"
                    disabled={loading}
                    onClick={() => void loadMore()}
                  >
                    加载更多事件
                  </Button>
                </div>
              )}
              {!readFailure && done && events.length > 0 && (
                <div className="flex justify-center pt-4 text-xs text-muted-foreground/70">
                  {newestFirst ? "— 已到最早 —" : "— 会话末尾 —"}
                </div>
              )}
            </div>
          </ScrollArea>

          {prompts && prompts.length > 0 && (
            <PromptTimeline
              prompts={prompts}
              activeIndex={activeTimelineIndex}
              onJump={jumpToTimelineMessage}
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
    <PreviewMutationDialogs
      provider={provider}
      fork={{
        target: forkTarget,
        running: forking,
        onClose: () => setForkTarget(null),
        onConfirm: () => void confirmForkAt(),
      }}
      edit={{
        target: editTarget,
        running: mutating,
        text: editText,
        onTextChange: setEditText,
        onClose: () => setEditTarget(null),
        onConfirm: () => void confirmEdit(),
      }}
      deleteEvent={{
        target: deleteTarget,
        plan: deletePlan,
        running: mutating,
        onClose: () => {
          setDeleteTarget(null);
          setDeletePlan(null);
        },
        onConfirm: () => void confirmDelete(),
      }}
      deleteSelection={{
        target: deleteSelectedTarget,
        plan: deletePlan,
        running: mutating,
        onClose: () => {
          setDeleteSelectedTarget(null);
          setDeletePlan(null);
        },
        onConfirm: () => void confirmDeleteSelected(),
      }}
    />
    <PreviewEditHistoryDialog
      open={historyOpen}
      history={editHistory}
      mutating={mutating}
      onOpenChange={setHistoryOpen}
      onUndo={() => void undoLastEdit()}
      onRestore={(snapshotName) => void restoreSnapshot(snapshotName)}
    />
    </>
  );
}

/* ---------- 单条事件（聊天气泡）---------- */

/**
 * 一轮里最终答复之前的过程性 Agent 消息。Codex App 不在对话流中展示这些消息，
 * 默认状态由“全部收起/全部展开”决定，同时允许当前会话中的每一轮单独切换。
 */
function ProcessTurnGroup({
  events,
  expanded,
  onExpandedChange,
  children,
}: {
  events: PreviewEvent[];
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  children: (event: PreviewEvent) => React.ReactNode;
}) {
  return (
    <div className="space-y-4">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => onExpandedChange(!expanded)}
        className="mx-auto flex items-center gap-1.5 rounded-full border border-border/60 bg-background/60 px-3 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Bot className="h-3 w-3" />
        <span>
          {expanded
            ? "收起本轮过程消息"
            : `已收起 ${events.length} 条过程消息`}
        </span>
        <ChevronDown
          className={cn("h-3 w-3 transition-transform", expanded && "rotate-180")}
        />
      </button>
      {expanded && (
        <div className="space-y-4 border-l-2 border-border/50 pl-3 opacity-90">
          {events.map((event) => children(event))}
        </div>
      )}
    </div>
  );
}

function EventBubble({ e, actions, query, disclosureMemory }: { e: PreviewEvent; actions: NodeActionSet; query: string; disclosureMemory: DisclosureMemory }) {
  const disclosureContext = useMemo(() => ({ ...disclosureMemory, eventIndex: e.index }), [disclosureMemory, e.index]);
  return <SearchHighlightContext.Provider value={query}><SearchDisclosureContext.Provider value={disclosureContext}><EventBubbleContent e={e} actions={actions} /></SearchDisclosureContext.Provider></SearchHighlightContext.Provider>;
}

function EventBubbleContent({ e, actions }: { e: PreviewEvent; actions: NodeActionSet }) {
  const ts = formatTimeString(e.timestamp);

  if (e.role === "subagent") {
    return <SubagentEventBubble e={e} ts={ts} actions={actions} />;
  }
  if (isEventMessage(e)) {
    return <EventMessageBubble e={e} ts={ts} actions={actions} />;
  }
  if (e.role === "user") {
    return <UserBubble e={e} ts={ts} actions={actions} />;
  }
  if (e.role === "assistant") {
    return <AssistantBubble e={e} ts={ts} actions={actions} />;
  }
  if (e.role === "reasoning") {
    return <ReasoningBubble e={e} ts={ts} actions={actions} />;
  }
  if (e.role === "tool_call" || e.role === "tool_result") {
    return <ToolBubble e={e} ts={ts} actions={actions} />;
  }
  if (e.role === "meta") {
    return <MetaLine e={e} ts={ts} />;
  }
  return <DefaultBubble e={e} ts={ts} />;
}

function SubagentOverview({ items }: { items: RelatedSubagentSession[] }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="border-y border-border/70 bg-background/30" aria-label="子智能体概览">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-1 py-2.5 text-left text-xs hover:text-foreground"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <Network className="h-3.5 w-3.5 text-cyan-700 dark:text-cyan-400" />
        <span className="font-medium">子智能体</span>
        <span className="tabular-nums text-muted-foreground">{items.length}</span>
        <ChevronDown
          className={cn(
            "ml-auto h-3.5 w-3.5 text-muted-foreground transition-transform",
            open && "rotate-180",
          )}
        />
      </button>
      {open && (
        <div className="border-t border-border/60 px-1">
          {items.map((item) => (
            <div
              key={item.id}
              className="flex min-w-0 items-start gap-2 border-t border-border/40 py-2.5 first:border-t-0"
              style={{ paddingLeft: Math.min((item.relativeDepth - 1) * 18, 72) }}
            >
              <div className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-cyan-500/10 text-cyan-700 dark:text-cyan-400">
                <Bot className="h-3 w-3" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs">
                  <span className="font-medium">{item.nickname ?? "子智能体"}</span>
                  <span className="font-mono text-[10px] text-muted-foreground" title={item.id}>
                    {item.id.slice(0, 8)}
                  </span>
                  {item.role && (
                    <Badge variant="outline" className="h-4 px-1 py-0 text-[10px] font-normal">
                      {item.role}
                    </Badge>
                  )}
                  <span className="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground">
                    L{item.depth}
                  </span>
                </div>
                <div className="mt-0.5 truncate font-mono text-[11px] text-foreground/70" title={item.agentPath}>
                  {item.agentPath}
                </div>
                <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[10px] text-muted-foreground">
                  <span>开始 {absoluteTime(item.createdAt)}</span>
                  <span>最后活动 {absoluteTime(item.updatedAt)}</span>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function SubagentEventBubble({
  e,
  ts,
  actions,
}: {
  e: PreviewEvent;
  ts: string;
  actions: NodeActionSet;
}) {
  const { open, toggle } = useSearchDisclosure("subagent", e.raw as object);
  const eventTime = subagentEventTime(e, ts);
  return (
    <div className="group flex gap-3">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-cyan-500/10 text-cyan-700 dark:text-cyan-400">
        <Network className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex w-full items-start gap-2 border-l-2 border-cyan-500/30 bg-background/50 px-3 py-2 text-xs">
          <button
            type="button"
            onClick={toggle}
            className="flex min-w-0 flex-1 items-start gap-2 text-left"
            aria-expanded={open}
          >
            <ChevronDown
              className={cn(
                "mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform",
                open && "rotate-180",
              )}
            />
            <span className="shrink-0 font-medium">{subagentEventLabel(e)}</span>
            {e.text_summary && (
              <span className="min-w-0 flex-1 break-words text-muted-foreground">
                <SearchText>{e.text_summary}</SearchText>
              </span>
            )}
            {eventTime && (
              <span className="shrink-0 font-mono text-muted-foreground/70">{eventTime}</span>
            )}
          </button>
          <NodeActionButtons event={e} actions={actions} />
        </div>
        {open && (
          <div data-search-disclosure className="mt-1.5 overflow-auto border-l-2 border-border/70 bg-card p-3 text-xs">
            <SearchJson data={e.raw as object} />
          </div>
        )}
      </div>
    </div>
  );
}

function EventMessageBubble({
  e,
  ts,
  actions,
}: {
  e: PreviewEvent;
  ts: string;
  actions: NodeActionSet;
}) {
  const { open, toggle } = useSearchDisclosure("event", e.raw as object);
  return (
    <div className="group flex gap-3">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-sky-500/15 text-sky-600 dark:text-sky-400">
        <Sparkles className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex w-full items-center gap-2 rounded-md border bg-card px-3 py-2 text-left text-xs shadow-sm hover:bg-accent">
          <button
            onClick={toggle}
            aria-expanded={open}
            className="flex min-w-0 flex-1 items-center gap-2 text-left"
          >
            <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 transition-transform", open && "rotate-180")} />
            <span className="shrink-0 font-medium">{eventMessageLabel(e)}</span>
            <EventSourceBadge e={e} />
            <span className="min-w-0 flex-1 truncate text-muted-foreground">
              <SearchText>{e.text_summary || ""}</SearchText>
            </span>
            {ts && <span className="shrink-0 font-mono text-muted-foreground/70">{ts}</span>}
          </button>
          <NodeActionButtons event={e} actions={actions} />
        </div>
        {open && (
          <div data-search-disclosure className="mt-1.5 overflow-auto rounded-md border bg-card p-3 text-xs">
            <SearchJson data={e.raw as object} />
          </div>
        )}
      </div>
    </div>
  );
}

function UserBubble({ e, ts, actions }: { e: PreviewEvent; ts: string; actions: NodeActionSet }) {
  const text = extractText(e);
  const embeddedTranscript = parseEmbeddedTranscriptPrompt(text);
  if (embeddedTranscript) {
    return <EmbeddedTranscriptBubble e={e} ts={ts} prompt={embeddedTranscript} actions={actions} />;
  }

  const diffComments = parseDiffCommentPrompt(text);
  if (diffComments) {
    return <DiffCommentBubble e={e} ts={ts} prompt={diffComments} actions={actions} />;
  }

  const message = parseUserMessageAttachments(text);

  return (
    <div className="group flex justify-end gap-3">
      <div className="flex min-w-0 max-w-[85%] flex-col items-end overflow-hidden">
        <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <NodeActionButtons event={e} actions={actions} />
          <span>你</span>
          <EventSourceBadge e={e} />
          {ts && <span className="font-mono">· {ts}</span>}
        </div>
        <div className="chat-md max-w-full rounded-2xl rounded-tr-sm bg-primary px-4 py-2.5 text-primary-foreground">
          {message.markdown ? <SearchMarkdown>{message.markdown}</SearchMarkdown> : message.images.length === 0 ? (
            <span className="italic opacity-70">(空消息)</span>
          ) : null}
          <LocalImageAttachments images={message.images} />
        </div>
      </div>
      <Avatar role="user" />
    </div>
  );
}

function EmbeddedTranscriptBubble({
  e,
  ts,
  prompt,
  actions,
}: {
  e: PreviewEvent;
  ts: string;
  prompt: EmbeddedTranscriptPrompt;
  actions: NodeActionSet;
}) {
  const { open, toggle } = useSearchDisclosure("transcript", prompt.transcript);
  return (
    <div className="group flex justify-end gap-3">
      <div className="flex min-w-0 max-w-[85%] flex-col items-end overflow-hidden">
        <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <NodeActionButtons event={e} actions={actions} />
          <span>你</span>
          <EventSourceBadge e={e} />
          {ts && <span className="font-mono">· {ts}</span>}
        </div>

        <div className="flex w-full flex-col items-end gap-2">
          <div className="inline-flex h-7 items-center gap-1.5 rounded-full border bg-card px-3 text-xs text-muted-foreground shadow-sm">
            <MessageSquare className="h-3.5 w-3.5" />
            <span>自动评审上下文</span>
          </div>

          {prompt.request && (
            <div className="chat-md max-w-full rounded-2xl rounded-tr-sm bg-primary px-4 py-2.5 text-primary-foreground">
              <SearchMarkdown>{prompt.request}</SearchMarkdown>
            </div>
          )}

          <button
            type="button"
            onClick={toggle}
            aria-expanded={open}
            className="inline-flex h-7 max-w-full items-center gap-1.5 rounded-md border bg-card px-2.5 text-left text-xs text-muted-foreground shadow-sm hover:bg-accent"
          >
            <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 transition-transform", open && "rotate-180")} />
            <span className="truncate">嵌入会话历史</span>
          </button>

          {open && (
            <pre data-search-disclosure className="max-h-80 max-w-full overflow-auto rounded-md border bg-card p-3 text-left font-mono text-xs leading-relaxed text-card-foreground">
              <SearchText>{prompt.transcript}</SearchText>
            </pre>
          )}
        </div>
      </div>
      <Avatar role="user" />
    </div>
  );
}

function DiffCommentBubble({
  e,
  ts,
  prompt,
  actions,
}: {
  e: PreviewEvent;
  ts: string;
  prompt: DiffCommentPrompt;
  actions: NodeActionSet;
}) {
  const countLabel = `${prompt.comments.length} 条批注`;

  return (
    <div className="group flex justify-end gap-3">
      <div className="flex min-w-0 max-w-[85%] flex-col items-end overflow-hidden">
        <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <NodeActionButtons event={e} actions={actions} />
          <span>你</span>
          <EventSourceBadge e={e} />
          {ts && <span className="font-mono">· {ts}</span>}
        </div>

        <div className="flex w-full flex-col items-end gap-2">
          <div className="inline-flex h-7 items-center gap-1.5 rounded-full border bg-card px-3 text-xs text-muted-foreground shadow-sm">
            <MessageSquare className="h-3.5 w-3.5" />
            <span>{countLabel}</span>
          </div>

          <div className="flex w-full flex-col items-end gap-2">
            {prompt.comments.map((comment) => (
              <div
                key={comment.number}
                className="w-full max-w-[28rem] overflow-hidden rounded-xl border bg-card px-4 py-3 text-left text-sm text-card-foreground shadow-sm"
              >
                {comment.context && (
                  <p className="mb-2 line-clamp-3 text-xs leading-relaxed text-muted-foreground">
                    <SearchText>{comment.context}</SearchText>
                  </p>
                )}
                <div className="chat-md font-medium">
                  <SearchMarkdown>{comment.body}</SearchMarkdown>
                </div>
              </div>
            ))}

            {prompt.request && (
              <div className="chat-md max-w-full rounded-2xl rounded-tr-sm bg-primary px-4 py-2.5 text-primary-foreground">
                <SearchMarkdown>{prompt.request}</SearchMarkdown>
              </div>
            )}
          </div>
        </div>
      </div>
      <Avatar role="user" />
    </div>
  );
}

function AssistantBubble({
  e,
  ts,
  actions,
}: {
  e: PreviewEvent;
  ts: string;
  actions: NodeActionSet;
}) {
  const text = extractText(e);
  return (
    <div className="group flex gap-3">
      <Avatar role="assistant" />
      <div className="flex min-w-0 max-w-[85%] flex-col overflow-hidden">
        <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <span>Assistant</span>
          <EventSourceBadge e={e} />
          {ts && <span className="font-mono">· {ts}</span>}
          <NodeActionButtons event={e} actions={actions} />
        </div>
        <div className="chat-md max-w-full rounded-2xl rounded-tl-sm border bg-card px-4 py-3 text-card-foreground shadow-sm">
          {text ? <SearchMarkdown>{text}</SearchMarkdown> : (
            <span className="italic text-muted-foreground">(空消息)</span>
          )}
        </div>
      </div>
    </div>
  );
}

function ReasoningBubble({ e, ts, actions }: { e: PreviewEvent; ts: string; actions: NodeActionSet }) {
  const text = extractText(e);
  const { open, toggle } = useSearchDisclosure("reasoning", text);
  return (
    <div className="group flex gap-3">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted">
        <Sparkles className="h-4 w-4 text-muted-foreground/70" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <button
            onClick={toggle}
            aria-expanded={open}
            className="flex items-center gap-1.5 text-[11px] text-muted-foreground hover:text-foreground"
          >
            <ChevronDown className={cn("h-3 w-3 transition-transform", open && "rotate-180")} />
            推理过程
            {ts && <span className="font-mono">· {ts}</span>}
          </button>
          <NodeActionButtons event={e} actions={actions} />
        </div>
        {open && text && (
          <pre data-search-disclosure className="mt-1.5 whitespace-pre-wrap break-words rounded-md border border-dashed bg-muted/40 px-3 py-2 font-mono text-xs text-muted-foreground">
            <SearchText>{text}</SearchText>
          </pre>
        )}
      </div>
    </div>
  );
}

function ToolBubble({ e, ts, actions }: { e: PreviewEvent; ts: string; actions: NodeActionSet }) {
  const { open, toggle } = useSearchDisclosure("tool", e.raw as object);
  const isCall = e.role === "tool_call";
  return (
    <div className="group flex gap-3">
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-full",
          isCall ? "bg-purple-500/15 text-purple-600 dark:text-purple-400" : "bg-amber-500/15 text-amber-600 dark:text-amber-400",
        )}
      >
        {isCall ? <Wrench className="h-4 w-4" /> : <Terminal className="h-4 w-4" />}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex w-full items-center gap-2 rounded-md border bg-card px-3 py-2 text-left text-xs shadow-sm hover:bg-accent">
          <button
            onClick={toggle}
            aria-expanded={open}
            className="flex min-w-0 flex-1 items-center gap-2 text-left"
          >
            <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 transition-transform", open && "rotate-180")} />
            <span className="font-medium">{isCall ? "工具调用" : "工具返回"}</span>
            <span className="truncate font-mono text-muted-foreground">{e.kind}</span>
            <span className="ml-auto min-w-0 flex-1 truncate text-muted-foreground">
              <SearchText>{e.text_summary || ""}</SearchText>
            </span>
            {ts && <span className="shrink-0 font-mono text-muted-foreground/70">{ts}</span>}
          </button>
          <NodeActionButtons event={e} actions={actions} />
        </div>
        {open && (
          <div data-search-disclosure className="mt-1.5 overflow-auto rounded-md border bg-card p-3 text-xs">
            <SearchJson data={e.raw as object} />
          </div>
        )}
      </div>
    </div>
  );
}

function MetaLine({ e, ts }: { e: PreviewEvent; ts: string }) {
  return (
    <div className="my-2 flex items-center gap-3">
      <div className="h-px flex-1 bg-border" />
      <div className="flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
        <Badge variant="outline" className="h-5 font-normal">
          {e.kind}
        </Badge>
        {e.text_summary && <span className="truncate"><SearchText>{e.text_summary}</SearchText></span>}
        {ts && <span className="font-mono">{ts}</span>}
      </div>
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}

function DefaultBubble({ e, ts }: { e: PreviewEvent; ts: string }) {
  const { open, toggle } = useSearchDisclosure("raw", e.raw as object);
  return (
    <div className="flex gap-3">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-slate-500/15 text-slate-600 dark:text-slate-400">
        <FileJson className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1">
        <button
          onClick={toggle}
          aria-expanded={open}
          className="flex w-full items-center gap-2 rounded-md border bg-card px-3 py-2 text-left text-xs shadow-sm hover:bg-accent"
        >
          <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 transition-transform", open && "rotate-180")} />
          <Badge variant="outline" className="h-5 font-normal capitalize">
            {e.role}
          </Badge>
          <span className="truncate font-mono text-muted-foreground">{e.kind}</span>
          {ts && <span className="ml-auto shrink-0 font-mono text-muted-foreground/70">{ts}</span>}
        </button>
        {open && (
          <div data-search-disclosure className="mt-1.5 overflow-auto rounded-md border bg-card p-3 text-xs">
            <SearchJson data={e.raw as object} />
          </div>
        )}
      </div>
    </div>
  );
}

function NodeActionButtons({ event, actions }: { event: PreviewEvent; actions: NodeActionSet }) {
  const showFork = actions.fork.enabled;
  const showEdit = actions.edit.enabled && actions.edit.canEditText(event);
  const showDelete = actions.edit.enabled && actions.edit.canDelete(event);
  if (!showFork && !showEdit && !showDelete) return null;
  const btnClass =
    "h-5 shrink-0 gap-1 px-1.5 text-[11px] opacity-0 transition-opacity duration-150 pointer-events-none group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100";
  return (
    <span className="inline-flex shrink-0 items-center gap-0.5">
      {showFork && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className={btnClass}
          disabled={actions.fork.pending}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            actions.fork.onSelect(event);
          }}
        >
          <GitBranch className="h-3 w-3" />
          回溯
        </Button>
      )}
      {showEdit && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className={btnClass}
          disabled={actions.edit.pending}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            actions.edit.onEdit(event);
          }}
        >
          <Pencil className="h-3 w-3" />
          编辑
        </Button>
      )}
      {showDelete && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className={cn(btnClass, "text-destructive hover:text-destructive")}
          disabled={actions.edit.pending}
          onClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            actions.edit.onDelete(event);
          }}
        >
          <Trash2 className="h-3 w-3" />
          删除
        </Button>
      )}
    </span>
  );
}

function Avatar({ role }: { role: "user" | "assistant" }) {
  if (role === "user") {
    return (
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground">
        <User className="h-4 w-4" />
      </div>
    );
  }
  return (
    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-emerald-500/15 text-emerald-600 dark:text-emerald-400">
      <Bot className="h-4 w-4" />
    </div>
  );
}

function Dot() {
  return (
    <span
      aria-hidden="true"
      className="inline-block h-1 w-1 shrink-0 rounded-full bg-muted-foreground/40"
    />
  );
}

function EventSourceBadge({ e }: { e: PreviewEvent }) {
  const outer = rawType(e);
  const payload = payloadType(e);
  if (outer !== "event_msg" && outer !== "response_item") return null;
  if (payload !== "user_message" && payload !== "agent_message" && payload !== "message") return null;

  const title =
    outer === "event_msg"
      ? "事件流消息：官方聊天展示层使用的用户/助手事件"
      : "响应项消息：模型对话历史中的消息项";

  return (
    <Badge
      variant="outline"
      title={title}
      className="h-4 px-1 py-0 font-mono text-[10px] font-normal text-muted-foreground"
    >
      {outer}/{payload}
    </Badge>
  );
}
