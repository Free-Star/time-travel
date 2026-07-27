import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { TimelineItem } from "./media";
import { dateFromArchive, formatMediaBytes } from "./media";
import "./TimelineView.css";

type MediaViewerProps = {
  item: TimelineItem;
  onChange: (item: TimelineItem) => void;
  onClose: () => void;
  onError: (message: string) => void;
};

type JournalEntryPreview = {
  id: number;
  entryDate: string;
  title: string;
  content: string;
  path: string;
};

const fullDateFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});
const numberFormatter = new Intl.NumberFormat("zh-CN");

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

export default function MediaViewer({
  item,
  onChange,
  onClose,
  onError,
}: MediaViewerProps) {
  const [loading, setLoading] = useState(false);
  const [viewerError, setViewerError] = useState("");
  const [journalEntries, setJournalEntries] = useState<JournalEntryPreview[]>([]);

  useEffect(() => {
    setViewerError("");
    const date = item.capturedAt.slice(0, 10);
    invoke<JournalEntryPreview[]>("journal_entries_for_date", { date })
      .then(setJournalEntries)
      .catch(() => setJournalEntries([]));
  }, [item.id]);

  useEffect(() => {
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
  }, []);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      if (event.key === "ArrowLeft") void navigate("newer");
      if (event.key === "ArrowRight") void navigate("older");
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  });

  async function navigate(direction: "newer" | "older") {
    if (loading) return;
    setLoading(true);
    setViewerError("");
    try {
      const next = await invoke<TimelineItem | null>("timeline_neighbor", {
        mediaId: item.id,
        direction,
      });
      if (next) onChange(next);
    } catch (reason) {
      const message = String(reason);
      setViewerError(message);
      onError(message);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="viewer-backdrop" role="dialog" aria-modal="true" aria-label="媒体查看器">
      <button className="viewer-close" type="button" aria-label="关闭查看器" onClick={onClose}>
        ×
      </button>
      <button
        className="viewer-nav previous"
        type="button"
        aria-label="查看较新媒体"
        disabled={loading}
        onClick={() => navigate("newer")}
      >
        ‹
      </button>

      <div className="viewer-canvas">
        {item.mediaKind === "video" ? (
          <video
            key={item.id}
            src={convertFileSrc(item.path)}
            controls
            autoPlay
            preload="metadata"
            onError={() => setViewerError("当前视频编码无法由系统播放器直接预览。")}
          />
        ) : viewerError && item.thumbnailPath ? (
          <img src={convertFileSrc(item.thumbnailPath)} alt="" />
        ) : (
          <img
            key={item.id}
            src={convertFileSrc(item.path)}
            alt={item.relativePath}
            onError={() => setViewerError("原图格式暂不支持直接显示，已保留只读文件信息。")}
          />
        )}
        {loading && <div className="viewer-busy">正在打开…</div>}
        {viewerError && <div className="viewer-warning">{viewerError}</div>}
      </div>

      <aside className="viewer-details">
        <span className="section-label">{item.mediaKind === "video" ? "VIDEO" : "PHOTO"}</span>
        <h3>{item.relativePath.split(/[\\/]/).pop()}</h3>
        <p className="viewer-path">{item.relativePath}</p>
        <dl>
          <div>
            <dt>拍摄时间</dt>
            <dd>{fullDateFormatter.format(dateFromArchive(item.capturedAt))}</dd>
          </div>
          <div>
            <dt>时间依据</dt>
            <dd>{captureSourceLabel(item.capturedSource, item.capturedPrecision)}</dd>
          </div>
          <div>
            <dt>尺寸</dt>
            <dd>
              {item.width && item.height
                ? `${numberFormatter.format(item.width)} × ${numberFormatter.format(item.height)}`
                : "未记录"}
            </dd>
          </div>
          <div>
            <dt>文件大小</dt>
            <dd>{formatMediaBytes(item.sizeBytes)}</dd>
          </div>
          <div>
            <dt>位置</dt>
            <dd>
              {item.latitude != null && item.longitude != null
                ? `${item.latitude.toFixed(5)}, ${item.longitude.toFixed(5)}`
                : "无定位信息"}
            </dd>
          </div>
        </dl>
        {journalEntries.length > 0 && (
          <section className="viewer-journal">
            <span className="section-label">当日日记</span>
            {journalEntries.map((entry) => (
              <button
                key={entry.id}
                type="button"
                onClick={() => invoke("open_journal_in_obsidian", { path: entry.path }).catch((reason) => onError(String(reason)))}
              >
                <strong>{entry.title}</strong>
                <small>{entry.content.replace(/[#>*_`\[\]]/g, "").trim().slice(0, 80)}</small>
              </button>
            ))}
          </section>
        )}
        <p className="viewer-readonly">只读打开 · 不修改原始媒体</p>
      </aside>

      <button
        className="viewer-nav next"
        type="button"
        aria-label="查看较旧媒体"
        disabled={loading}
        onClick={() => navigate("older")}
      >
        ›
      </button>
    </div>
  );
}
