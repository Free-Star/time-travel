import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import "./TimelineView.css";

type TimelineMonth = {
  key: string;
  total: number;
  photos: number;
  videos: number;
  withLocation: number;
};

type TimelineItem = {
  id: number;
  path: string;
  relativePath: string;
  mediaKind: "photo" | "video";
  extension: string;
  sizeBytes: number;
  capturedAt: string;
  capturedSource: string;
  capturedPrecision: string;
  latitude: number | null;
  longitude: number | null;
  width: number | null;
  height: number | null;
  thumbnailPath: string | null;
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
const fullDateFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});
const numberFormatter = new Intl.NumberFormat("zh-CN");

function dateFromArchive(value: string) {
  return new Date(value.includes("T") ? value : `${value}T12:00:00`);
}

function formatMonth(key: string) {
  return monthFormatter.format(new Date(`${key}-01T12:00:00`));
}

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function captureSourceLabel(source: string, precision: string) {
  const sourceLabels: Record<string, string> = {
    exif: "照片 EXIF",
    filename: "文件名",
    folder: "归档目录",
    modified: "文件时间",
  };
  const precisionLabels: Record<string, string> = {
    second: "精确到秒",
    day: "精确到天",
    month: "精确到月",
  };
  return `${sourceLabels[source] ?? source} · ${precisionLabels[precision] ?? precision}`;
}

export default function TimelineView({ totalMedia, onError }: TimelineViewProps) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const requestSequence = useRef(0);
  const [months, setMonths] = useState<TimelineMonth[]>([]);
  const [selectedMonth, setSelectedMonth] = useState("");
  const [timelineWindow, setTimelineWindow] = useState<TimelineWindow | null>(null);
  const [viewport, setViewport] = useState({ width: 760, height: 600, scrollTop: 0 });
  const [loading, setLoading] = useState(true);
  const [generating, setGenerating] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [viewer, setViewer] = useState<TimelineItem | null>(null);
  const [viewerLoading, setViewerLoading] = useState(false);
  const [viewerError, setViewerError] = useState("");
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

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!viewer) return;
      if (event.key === "Escape") setViewer(null);
      if (event.key === "ArrowLeft") void navigateViewer("newer");
      if (event.key === "ArrowRight") void navigateViewer("older");
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  });

  useEffect(() => {
    if (!viewer) return;
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
  }, [viewer]);

  const visibleMissingIds = useMemo(
    () =>
      (timelineWindow?.items ?? [])
        .filter((item) => !item.thumbnailPath)
        .slice(0, 24)
        .map((item) => item.id),
    [timelineWindow],
  );
  const renderedItems =
    timelineWindow?.month === selectedMonth ? timelineWindow.items : [];
  const renderedOffset =
    timelineWindow?.month === selectedMonth ? timelineWindow.offset : windowOffset;

  function chooseMonth(key: string) {
    setSelectedMonth(key);
    setTimelineWindow(null);
    setFailedImages(new Set());
    const element = viewportRef.current;
    if (element) element.scrollTop = 0;
    setViewport((current) => ({ ...current, scrollTop: 0 }));
  }

  async function generateVisiblePreviews() {
    if (!visibleMissingIds.length) return;
    setGenerating(true);
    onError("");
    try {
      const report = await invoke<ThumbnailReport>("generate_timeline_thumbnails", {
        mediaIds: visibleMissingIds,
      });
      setRefreshKey((value) => value + 1);
      if (report.failed) {
        onError(`本屏预览生成完成，其中 ${report.failed} 个格式暂不支持。`);
      }
    } catch (reason) {
      onError(String(reason));
    } finally {
      setGenerating(false);
    }
  }

  async function stopGenerating() {
    await invoke<boolean>("cancel_thumbnails");
  }

  async function openViewer(mediaId: number) {
    setViewerLoading(true);
    setViewerError("");
    try {
      setViewer(await invoke<TimelineItem>("open_timeline_media", { mediaId }));
    } catch (reason) {
      onError(String(reason));
    } finally {
      setViewerLoading(false);
    }
  }

  async function navigateViewer(direction: "newer" | "older") {
    if (!viewer || viewerLoading) return;
    setViewerLoading(true);
    setViewerError("");
    try {
      const next = await invoke<TimelineItem | null>("timeline_neighbor", {
        mediaId: viewer.id,
        direction,
      });
      if (next) setViewer(next);
    } catch (reason) {
      setViewerError(String(reason));
    } finally {
      setViewerLoading(false);
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
            {visibleMissingIds.length > 0 && (
              <button
                className="secondary-button timeline-cache-button"
                type="button"
                disabled={viewerLoading}
                onClick={generating ? stopGenerating : generateVisiblePreviews}
              >
                {generating
                  ? "停止生成"
                  : `补全本屏预览 · ${visibleMissingIds.length}`}
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
                            loading="lazy"
                            onError={() => markImageFailed(item.id)}
                          />
                        ) : (
                          <span className="media-placeholder">
                            <b>{item.mediaKind === "video" ? "▶" : "◇"}</b>
                            <small>{item.extension.toUpperCase()}</small>
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
        <div className="viewer-backdrop" role="dialog" aria-modal="true" aria-label="媒体查看器">
          <button
            className="viewer-close"
            type="button"
            aria-label="关闭查看器"
            onClick={() => setViewer(null)}
          >
            ×
          </button>
          <button
            className="viewer-nav previous"
            type="button"
            aria-label="查看较新媒体"
            disabled={viewerLoading}
            onClick={() => navigateViewer("newer")}
          >
            ‹
          </button>

          <div className="viewer-canvas">
            {viewer.mediaKind === "video" ? (
              <video
                key={viewer.id}
                src={convertFileSrc(viewer.path)}
                controls
                autoPlay
                preload="metadata"
                onError={() => setViewerError("当前视频编码无法由系统播放器直接预览。")}
              />
            ) : viewerError && viewer.thumbnailPath ? (
              <img src={convertFileSrc(viewer.thumbnailPath)} alt="" />
            ) : (
              <img
                key={viewer.id}
                src={convertFileSrc(viewer.path)}
                alt={viewer.relativePath}
                onError={() =>
                  setViewerError("原图格式暂不支持直接显示，已保留只读文件信息。")
                }
              />
            )}
            {viewerLoading && <div className="viewer-busy">正在打开…</div>}
            {viewerError && <div className="viewer-warning">{viewerError}</div>}
          </div>

          <aside className="viewer-details">
            <span className="section-label">
              {viewer.mediaKind === "video" ? "VIDEO" : "PHOTO"}
            </span>
            <h3>{viewer.relativePath.split(/[\\/]/).pop()}</h3>
            <p className="viewer-path">{viewer.relativePath}</p>
            <dl>
              <div>
                <dt>拍摄时间</dt>
                <dd>{fullDateFormatter.format(dateFromArchive(viewer.capturedAt))}</dd>
              </div>
              <div>
                <dt>时间依据</dt>
                <dd>
                  {captureSourceLabel(viewer.capturedSource, viewer.capturedPrecision)}
                </dd>
              </div>
              <div>
                <dt>尺寸</dt>
                <dd>
                  {viewer.width && viewer.height
                    ? `${numberFormatter.format(viewer.width)} × ${numberFormatter.format(viewer.height)}`
                    : "未记录"}
                </dd>
              </div>
              <div>
                <dt>文件大小</dt>
                <dd>{formatBytes(viewer.sizeBytes)}</dd>
              </div>
              <div>
                <dt>位置</dt>
                <dd>
                  {viewer.latitude != null && viewer.longitude != null
                    ? `${viewer.latitude.toFixed(5)}, ${viewer.longitude.toFixed(5)}`
                    : "无定位信息"}
                </dd>
              </div>
            </dl>
            <p className="viewer-readonly">只读打开 · 不修改原始媒体</p>
          </aside>

          <button
            className="viewer-nav next"
            type="button"
            aria-label="查看较旧媒体"
            disabled={viewerLoading}
            onClick={() => navigateViewer("older")}
          >
            ›
          </button>
        </div>
      )}
    </>
  );
}
