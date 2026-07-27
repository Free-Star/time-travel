import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import MediaViewer from "./MediaViewer";
import type { TimelineItem } from "./media";
import { dateFromArchive } from "./media";
import "./TimelineView.css";

type TimelineMonth = {
  key: string;
  total: number;
  photos: number;
  videos: number;
  withLocation: number;
};

type TimelineWindow = {
  month: string;
  total: number;
  offset: number;
  items: TimelineItem[];
};

type ThumbnailReport = {
  status: "completed" | "cancelled";
  processed: number;
  ready: number;
  failed: number;
};

type ThumbnailResult = {
  mediaId: number;
  status: "ready" | "failed";
  cachePath: string | null;
};

type ThumbnailProgress = {
  status: "generating" | "completed" | "cancelled";
};

type TimelineViewProps = {
  totalMedia: number;
  onError: (message: string) => void;
};

const TILE_GAP = 10;
const MIN_TILE_WIDTH = 142;
const OVERSCAN_ROWS = 2;

const monthFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "long",
});
const dayFormatter = new Intl.DateTimeFormat("zh-CN", {
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});
const numberFormatter = new Intl.NumberFormat("zh-CN");

function formatMonth(key: string) {
  return monthFormatter.format(new Date(`${key}-01T12:00:00`));
}

export default function TimelineView({ totalMedia, onError }: TimelineViewProps) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const requestSequence = useRef(0);
  const automaticJob = useRef(false);
  const [months, setMonths] = useState<TimelineMonth[]>([]);
  const [selectedMonth, setSelectedMonth] = useState("");
  const [timelineWindow, setTimelineWindow] = useState<TimelineWindow | null>(null);
  const [viewport, setViewport] = useState({ width: 760, height: 600, scrollTop: 0 });
  const [loading, setLoading] = useState(true);
  const [generating, setGenerating] = useState(false);
  const [queueTick, setQueueTick] = useState(0);
  const [refreshKey, setRefreshKey] = useState(0);
  const [viewer, setViewer] = useState<TimelineItem | null>(null);
  const [failedImages, setFailedImages] = useState<Set<number>>(() => new Set());

  useEffect(() => {
    let active = true;
    invoke<TimelineMonth[]>("timeline_months")
      .then((result) => {
        if (!active) return;
        setMonths(result);
        setSelectedMonth((current) => current || result[0]?.key || "");
      })
      .catch((reason) => onError(String(reason)))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [onError]);

  useEffect(() => {
    const unlisten = listen<ThumbnailResult>("thumbnail-result", (event) => {
      const result = event.payload;
      setTimelineWindow((current) =>
        current
          ? {
              ...current,
              items: current.items.map((item) =>
                item.id === result.mediaId
                  ? {
                      ...item,
                      thumbnailPath: result.cachePath,
                      thumbnailStatus: result.status,
                    }
                  : item,
              ),
            }
          : current,
      );
      if (result.status === "ready") {
        setFailedImages((current) => {
          if (!current.has(result.mediaId)) return current;
          const next = new Set(current);
          next.delete(result.mediaId);
          return next;
        });
      }
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<ThumbnailProgress>("thumbnail-progress", (event) => {
      if (event.payload.status !== "generating") {
        setQueueTick((value) => value + 1);
      }
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  useLayoutEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    const updateSize = () => {
      setViewport((current) => ({
        ...current,
        width: element.clientWidth,
        height: element.clientHeight,
      }));
    };
    updateSize();
    const observer = new ResizeObserver(updateSize);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const columns = Math.max(
    3,
    Math.min(8, Math.floor((viewport.width + TILE_GAP) / (MIN_TILE_WIDTH + TILE_GAP))),
  );
  const tileWidth = Math.max(
    110,
    (viewport.width - TILE_GAP * Math.max(0, columns - 1) - 2) / columns,
  );
  const rowHeight = tileWidth + 42 + TILE_GAP;
  const selectedSummary = months.find((month) => month.key === selectedMonth);
  const totalRows = Math.ceil((selectedSummary?.total ?? 0) / columns);
  const startRow = Math.max(0, Math.floor(viewport.scrollTop / rowHeight) - OVERSCAN_ROWS);
  const visibleRows =
    Math.ceil(viewport.height / rowHeight) + OVERSCAN_ROWS * 2;
  const windowOffset = startRow * columns;
  const windowLimit = Math.min(500, visibleRows * columns);

  useEffect(() => {
    if (!selectedMonth) return;
    const sequence = ++requestSequence.current;
    const timer = window.setTimeout(() => {
      invoke<TimelineWindow>("timeline_window", {
        month: selectedMonth,
        offset: windowOffset,
        limit: windowLimit,
      })
        .then((result) => {
          if (sequence === requestSequence.current) setTimelineWindow(result);
        })
        .catch((reason) => onError(String(reason)));
    }, 35);
    return () => window.clearTimeout(timer);
  }, [onError, refreshKey, selectedMonth, windowLimit, windowOffset]);

  const prioritizedItems = useMemo(() => {
    if (!timelineWindow || timelineWindow.month !== selectedMonth) return [];
    const firstVisibleOffset = Math.floor(viewport.scrollTop / rowHeight) * columns;
    const firstVisibleIndex = Math.max(0, firstVisibleOffset - timelineWindow.offset);
    const visibleCount = (Math.ceil(viewport.height / rowHeight) + 1) * columns;
    const end = Math.min(timelineWindow.items.length, firstVisibleIndex + visibleCount);
    return [
      ...timelineWindow.items.slice(firstVisibleIndex, end),
      ...timelineWindow.items.slice(end),
      ...timelineWindow.items.slice(0, firstVisibleIndex).reverse(),
    ];
  }, [
    columns,
    rowHeight,
    selectedMonth,
    timelineWindow,
    viewport.height,
    viewport.scrollTop,
  ]);
  const automaticIds = useMemo(
    () =>
      prioritizedItems
        .filter((item) => !item.thumbnailPath && item.thumbnailStatus !== "failed")
        // Visible cards come first; the remainder warms the neighboring window
        // while the user is reading the current screen.
        .slice(0, 72)
        .map((item) => item.id),
    [prioritizedItems],
  );
  const retryIds = useMemo(
    () =>
      prioritizedItems
        .filter((item) => !item.thumbnailPath)
        .slice(0, 36)
        .map((item) => item.id),
    [prioritizedItems],
  );
  const failedPreviewCount = prioritizedItems.filter(
    (item) => item.thumbnailStatus === "failed",
  ).length;
  const automaticKey = automaticIds.join(",");
  const renderedItems =
    timelineWindow?.month === selectedMonth ? timelineWindow.items : [];
  const renderedOffset =
    timelineWindow?.month === selectedMonth ? timelineWindow.offset : windowOffset;

  useEffect(() => {
    if (!automaticKey || automaticJob.current) return;
    const mediaIds = automaticIds;
    const timer = window.setTimeout(() => {
      automaticJob.current = true;
      setGenerating(true);
      invoke<ThumbnailReport | null>("ensure_timeline_thumbnails", { mediaIds })
        .then((report) => {
          window.setTimeout(
            () => setQueueTick((value) => value + 1),
            report ? 0 : 400,
          );
        })
        .catch((reason) => onError(String(reason)))
        .finally(() => {
          automaticJob.current = false;
          setGenerating(false);
        });
    }, 160);
    return () => window.clearTimeout(timer);
  }, [automaticKey, automaticIds, onError, queueTick]);

  function chooseMonth(key: string) {
    setSelectedMonth(key);
    setTimelineWindow(null);
    setFailedImages(new Set());
    const element = viewportRef.current;
    if (element) element.scrollTop = 0;
    setViewport((current) => ({ ...current, scrollTop: 0 }));
  }

  async function generateVisiblePreviews() {
    if (!retryIds.length || automaticJob.current) return;
    automaticJob.current = true;
    setGenerating(true);
    onError("");
    try {
      const report = await invoke<ThumbnailReport>("generate_timeline_thumbnails", {
        mediaIds: retryIds,
      });
      if (report.failed) {
        onError(`本屏预览生成完成，其中 ${report.failed} 个格式暂不支持。`);
      }
    } catch (reason) {
      onError(String(reason));
    } finally {
      automaticJob.current = false;
      setGenerating(false);
      setRefreshKey((value) => value + 1);
      setQueueTick((value) => value + 1);
    }
  }

  async function openViewer(mediaId: number) {
    try {
      setViewer(await invoke<TimelineItem>("open_timeline_media", { mediaId }));
    } catch (reason) {
      onError(String(reason));
    }
  }

  function markImageFailed(mediaId: number) {
    setFailedImages((current) => new Set(current).add(mediaId));
  }

  if (loading) {
    return (
      <section className="timeline-loading">
        <span className="scan-pulse" />
        正在整理时间线…
      </section>
    );
  }

  return (
    <>
      <section className="timeline-shell">
        <aside className="month-rail" aria-label="相册月份">
          <div className="month-rail-heading">
            <span>时间索引</span>
            <strong>{numberFormatter.format(totalMedia)}</strong>
          </div>
          <div className="month-list">
            {months.map((month) => (
              <button
                className={month.key === selectedMonth ? "active" : ""}
                key={month.key}
                type="button"
                onClick={() => chooseMonth(month.key)}
              >
                <span>{formatMonth(month.key)}</span>
                <small>{numberFormatter.format(month.total)}</small>
              </button>
            ))}
          </div>
        </aside>

        <div className="timeline-stage">
          <header className="timeline-heading">
            <div>
              <span className="section-label">VIRTUAL TIMELINE</span>
              <h2>{selectedMonth ? formatMonth(selectedMonth) : "时间线"}</h2>
              <p>
                {selectedSummary
                  ? `${numberFormatter.format(selectedSummary.photos)} 张照片 · ${numberFormatter.format(selectedSummary.videos)} 段视频 · ${numberFormatter.format(selectedSummary.withLocation)} 个带定位`
                  : "没有可显示的媒体"}
              </p>
            </div>
            {generating && (
              <div className="timeline-realtime-status" aria-live="polite">
                <span />
                实时生成预览
              </div>
            )}
            {!generating && failedPreviewCount > 0 && (
              <button
                className="secondary-button timeline-cache-button"
                type="button"
                onClick={generateVisiblePreviews}
              >
                重试失败预览 · {failedPreviewCount}
              </button>
            )}
          </header>

          <div
            className="virtual-viewport"
            ref={viewportRef}
            onScroll={(event) => {
              const scrollTop = event.currentTarget.scrollTop;
              setViewport((current) => ({
                ...current,
                scrollTop,
              }));
            }}
          >
            <div
              className="virtual-spacer"
              style={{ height: Math.max(viewport.height, totalRows * rowHeight) }}
            >
              <div
                className="timeline-grid"
                style={{
                  gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                  top: Math.floor(renderedOffset / columns) * rowHeight,
                }}
              >
                {renderedItems.map((item) => {
                  const imageFailed = failedImages.has(item.id);
                  return (
                    <button
                      className="timeline-tile"
                      key={item.id}
                      type="button"
                      onClick={() => openViewer(item.id)}
                    >
                      <span className="timeline-image">
                        {item.thumbnailPath && !imageFailed ? (
                          <img
                            src={convertFileSrc(item.thumbnailPath)}
                            alt=""
                            loading="eager"
                            onError={() => markImageFailed(item.id)}
                          />
                        ) : (
                          <span
                            className={`media-placeholder ${
                              item.thumbnailStatus === "failed" ? "is-failed" : "is-pending"
                            }`}
                          >
                            <b>{item.mediaKind === "video" ? "▶" : "◇"}</b>
                            <small>
                              {item.thumbnailStatus === "failed"
                                ? `${item.extension.toUpperCase()} · 暂无预览`
                                : item.extension.toUpperCase()}
                            </small>
                          </span>
                        )}
                        {item.mediaKind === "video" && <i className="tile-video-badge">▶</i>}
                      </span>
                      <span className="tile-caption">
                        <strong>{dayFormatter.format(dateFromArchive(item.capturedAt))}</strong>
                        <small>{item.relativePath.split(/[\\/]/).pop()}</small>
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          </div>
          <footer className="virtual-status">
            <span>
              当前仅渲染 {numberFormatter.format(renderedItems.length)} 项
            </span>
            <span>
              第 {numberFormatter.format(Math.min(renderedOffset + 1, selectedSummary?.total ?? 0))}–
              {numberFormatter.format(
                Math.min(
                  renderedOffset + renderedItems.length,
                  selectedSummary?.total ?? 0,
                ),
              )}{" "}
              项
            </span>
          </footer>
        </div>
      </section>

      {viewer && (
        <MediaViewer
          item={viewer}
          onChange={setViewer}
          onClose={() => setViewer(null)}
          onError={onError}
        />
      )}
    </>
  );
}
