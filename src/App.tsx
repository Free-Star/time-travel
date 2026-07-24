import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import "./App.css";
import TimelineView from "./TimelineView";

type LibrarySummary = {
  root: string;
  displayName: string;
  topLevelFolders: number;
  topLevelMedia: number;
  projectDirectoryExcluded: boolean;
  writePolicy: string;
};

type IndexSummary = {
  total: number;
  photos: number;
  videos: number;
  withLocation: number;
  lastScanAt: string | null;
  needsMetadataRefresh: boolean;
};

type ScanProgress = {
  status: "scanning" | "completed" | "cancelled";
  discovered: number;
  inserted: number;
  updated: number;
  unchanged: number;
  errors: number;
  currentPath: string;
};

type ScanReport = ScanProgress & {
  removed: number;
  summary: IndexSummary;
};

type ThumbnailStatus = {
  totalMedia: number;
  ready: number;
  failed: number;
  cacheBytes: number;
  ffmpegAvailable: boolean;
};

type ThumbnailPreview = {
  mediaId: number;
  mediaKind: "photo" | "video";
  capturedAt: string;
  cachePath: string;
};

type ThumbnailProgress = {
  status: "generating" | "completed" | "cancelled";
  total: number;
  processed: number;
  ready: number;
  failed: number;
  currentPath: string;
};

type ThumbnailReport = {
  status: string;
  processed: number;
  ready: number;
  failed: number;
  thumbnailStatus: ThumbnailStatus;
  previews: ThumbnailPreview[];
};

function App() {
  const [view, setView] = useState<"home" | "timeline">("home");
  const [library, setLibrary] = useState<LibrarySummary | null>(null);
  const [index, setIndex] = useState<IndexSummary | null>(null);
  const [scanProgress, setScanProgress] = useState<ScanProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [thumbnailStatus, setThumbnailStatus] = useState<ThumbnailStatus | null>(null);
  const [thumbnailPreviews, setThumbnailPreviews] = useState<ThumbnailPreview[]>([]);
  const [thumbnailProgress, setThumbnailProgress] = useState<ThumbnailProgress | null>(null);
  const [generatingThumbnails, setGeneratingThumbnails] = useState(false);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    invoke<LibrarySummary | null>("current_library")
      .then((summary) => {
        setLibrary(summary);
        if (summary) return loadLibraryData();
      })
      .catch((reason) => setError(String(reason)))
      .finally(() => setBusy(false));
  }, []);

  useEffect(() => {
    const unlisten = listen<ScanProgress>("scan-progress", (event) => {
      setScanProgress(event.payload);
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<ThumbnailProgress>("thumbnail-progress", (event) => {
      setThumbnailProgress(event.payload);
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  async function loadLibraryData() {
    await Promise.all([loadIndex(), loadThumbnailData()]);
  }

  async function loadIndex() {
    const summary = await invoke<IndexSummary | null>("current_index");
    setIndex(summary);
  }

  async function loadThumbnailData() {
    const [status, previews] = await Promise.all([
      invoke<ThumbnailStatus | null>("thumbnail_status"),
      invoke<ThumbnailPreview[]>("thumbnail_previews", { limit: 12 }),
    ]);
    setThumbnailStatus(status);
    setThumbnailPreviews(previews);
  }

  async function chooseLibrary() {
    setError("");
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择只读相册目录",
    });

    if (!selected) return;

    setBusy(true);
    try {
      const summary = await invoke<LibrarySummary>("configure_library", {
        root: selected,
      });
      setLibrary(summary);
      await loadLibraryData();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function startScan() {
    setError("");
    setScanning(true);
    setScanProgress({
      status: "scanning",
      discovered: 0,
      inserted: 0,
      updated: 0,
      unchanged: 0,
      errors: 0,
      currentPath: "",
    });
    try {
      const report = await invoke<ScanReport>("scan_library");
      setIndex(report.summary);
      setScanProgress(report);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setScanning(false);
    }
  }

  async function cancelScan() {
    await invoke<boolean>("cancel_scan");
  }

  async function generateThumbnailBatch() {
    setError("");
    setGeneratingThumbnails(true);
    setThumbnailProgress({
      status: "generating",
      total: 0,
      processed: 0,
      ready: 0,
      failed: 0,
      currentPath: "",
    });
    try {
      const report = await invoke<ThumbnailReport>("generate_thumbnails", { limit: 30 });
      setThumbnailStatus(report.thumbnailStatus);
      setThumbnailPreviews(report.previews);
      setThumbnailProgress((current) =>
        current
          ? { ...current, status: report.status as ThumbnailProgress["status"] }
          : current,
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setGeneratingThumbnails(false);
    }
  }

  async function cancelThumbnailBatch() {
    await invoke<boolean>("cancel_thumbnails");
  }

  async function clearThumbnailCache() {
    if (!window.confirm("只清除时空相册生成的预览缓存，原始媒体不会受到影响。继续吗？")) {
      return;
    }
    setError("");
    try {
      const status = await invoke<ThumbnailStatus>("clear_thumbnail_cache");
      setThumbnailStatus(status);
      setThumbnailPreviews([]);
      setThumbnailProgress(null);
    } catch (reason) {
      setError(String(reason));
    }
  }

  function formatBytes(bytes: number) {
    if (bytes < 1024 * 1024) return `${Math.max(0, Math.round(bytes / 1024))} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }

  const number = new Intl.NumberFormat("zh-CN");

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">时</span>
          <div>
            <strong>时空相册</strong>
            <small>个人记忆库</small>
          </div>
        </div>

        <nav aria-label="主导航">
          <button
            className={`nav-item ${view === "home" ? "active" : ""}`}
            type="button"
            onClick={() => {
              setView("home");
              if (library) void loadThumbnailData();
            }}
          >
            <span>⌁</span>开始
          </button>
          <button
            className={`nav-item ${view === "timeline" ? "active" : ""}`}
            type="button"
            disabled={!index?.total || index.needsMetadataRefresh}
            onClick={() => {
              setError("");
              setView("timeline");
            }}
          >
            <span>◷</span>时间线
          </button>
          <button className="nav-item" type="button" disabled>
            <span>⌖</span>地图
          </button>
        </nav>

        <div className="sidebar-foot">
          <span className="status-dot" />
          本地运行 · 不上传
        </div>
      </aside>

      <main>
        <header className="topbar">
          <div>
            <span className="eyebrow">阶段 4 · 时间线与查看器</span>
            <h1>
              {view === "timeline"
                ? "沿着时间，重看记忆"
                : library
                  ? "相册已连接"
                  : "连接你的相册库"}
            </h1>
          </div>
          <div className="readonly-pill">只读模式</div>
        </header>

        {view === "timeline" && error && (
          <button className="global-error" type="button" onClick={() => setError("")}>
            {error}
            <span>×</span>
          </button>
        )}

        {view === "home" ? (
          <section className="content">
          <div className="hero-card">
            <div className="hero-copy">
              <span className="section-label">READ-ONLY BY DESIGN</span>
              <h2>
                只观察记忆，
                <br />
                不改变原件。
              </h2>
              <p>
                时空相册只读取媒体和元数据。索引、缩略图与所有人工修正都会保存在相册目录之外。
              </p>

              <button
                className="primary-button"
                type="button"
                onClick={chooseLibrary}
                disabled={busy}
              >
                {busy ? "正在确认…" : library ? "更换相册目录" : "选择相册目录"}
                <span>→</span>
              </button>

              {error && <p className="error-message">{error}</p>}
            </div>

            <div className="safety-panel">
              <div className="shield">✓</div>
              <h3>媒体安全边界</h3>
              <ul>
                <li>
                  <span>✓</span>不写入文件内容或 EXIF
                </li>
                <li>
                  <span>✓</span>不移动、重命名或删除
                </li>
                <li>
                  <span>✓</span>写入目标位于相册内时立即拒绝
                </li>
                <li>
                  <span>✓</span>开发目录自动排除扫描
                </li>
              </ul>
            </div>
          </div>

          {library && (
            <section className="library-card" aria-live="polite">
              <div>
                <span className="section-label">当前数据源</span>
                <h3>{library.displayName}</h3>
                <code>{library.root}</code>
              </div>
              <dl>
                <div>
                  <dt>一级目录</dt>
                  <dd>{library.topLevelFolders}</dd>
                </div>
                <div>
                  <dt>根目录媒体</dt>
                  <dd>{library.topLevelMedia}</dd>
                </div>
                <div>
                  <dt>开发目录</dt>
                  <dd>{library.projectDirectoryExcluded ? "已排除" : "不在相册内"}</dd>
                </div>
                <div>
                  <dt>写入策略</dt>
                  <dd>{library.writePolicy}</dd>
                </div>
              </dl>
              <p className="next-step">
                扫描只读取路径、文件属性与照片 EXIF；数据库位于系统应用数据目录。
              </p>
            </section>
          )}

          {library && (
            <section className="scan-card">
              <div className="scan-heading">
                <div>
                  <span className="section-label">本地媒体索引</span>
                  <h3>{index?.total ? `${number.format(index.total)} 个媒体` : "尚未建立索引"}</h3>
                  <p>
                    {index?.needsMetadataRefresh
                      ? "日期识别规则已更新，请执行一次只读索引刷新后再进入时间线。"
                      : index?.lastScanAt
                        ? `上次完成：${new Date(index.lastScanAt).toLocaleString("zh-CN")}`
                        : "首次扫描会读取媒体元数据，但不会生成缩略图。"}
                  </p>
                </div>
                <div className="scan-actions">
                  {scanning ? (
                    <button className="secondary-button" type="button" onClick={cancelScan}>
                      停止扫描
                    </button>
                  ) : (
                    <button className="primary-button compact" type="button" onClick={startScan}>
                      {index?.needsMetadataRefresh
                        ? "更新元数据索引"
                        : index?.total
                          ? "增量扫描"
                          : "开始只读扫描"}
                      <span>→</span>
                    </button>
                  )}
                </div>
              </div>

              {(index?.total || scanProgress) && (
                <div className="index-stats">
                  <div>
                    <span>照片</span>
                    <strong>{number.format(index?.photos ?? scanProgress?.inserted ?? 0)}</strong>
                  </div>
                  <div>
                    <span>视频</span>
                    <strong>{number.format(index?.videos ?? 0)}</strong>
                  </div>
                  <div>
                    <span>带定位</span>
                    <strong>{number.format(index?.withLocation ?? 0)}</strong>
                  </div>
                  <div>
                    <span>{scanning ? "已发现" : "错误"}</span>
                    <strong>
                      {number.format(
                        scanning ? (scanProgress?.discovered ?? 0) : (scanProgress?.errors ?? 0),
                      )}
                    </strong>
                  </div>
                </div>
              )}

              {scanProgress && (
                <div className={`scan-progress ${scanProgress.status}`}>
                  <span className="scan-pulse" />
                  <div>
                    <strong>
                      {scanProgress.status === "scanning"
                        ? `正在建立索引 · ${number.format(scanProgress.discovered)}`
                        : scanProgress.status === "completed"
                          ? "扫描完成"
                          : "扫描已停止"}
                    </strong>
                    <small>
                      新增 {number.format(scanProgress.inserted)} · 更新{" "}
                      {number.format(scanProgress.updated)} · 未变化{" "}
                      {number.format(scanProgress.unchanged)} · 错误{" "}
                      {number.format(scanProgress.errors)}
                    </small>
                  </div>
                </div>
              )}
            </section>
          )}

          {library && index?.total ? (
            <section className="thumbnail-card">
              <div className="scan-heading">
                <div>
                  <span className="section-label">缩略图与视频封面</span>
                  <h3>
                    {number.format(thumbnailStatus?.ready ?? 0)} / {number.format(index.total)}{" "}
                    已就绪
                  </h3>
                  <p>
                    缓存 {formatBytes(thumbnailStatus?.cacheBytes ?? 0)} · FFmpeg{" "}
                    {thumbnailStatus?.ffmpegAvailable ? "可用" : "不可用"}
                  </p>
                </div>
                <div className="scan-actions">
                  {generatingThumbnails ? (
                    <button
                      className="secondary-button"
                      type="button"
                      onClick={cancelThumbnailBatch}
                    >
                      停止生成
                    </button>
                  ) : (
                    <button
                      className="primary-button compact"
                      type="button"
                      onClick={generateThumbnailBatch}
                    >
                      生成下一批 30 个
                      <span>→</span>
                    </button>
                  )}
                  {(thumbnailStatus?.ready ?? 0) > 0 && !generatingThumbnails && (
                    <button className="text-button" type="button" onClick={clearThumbnailCache}>
                      清理缓存
                    </button>
                  )}
                </div>
              </div>

              {thumbnailProgress && (
                <div className={`scan-progress ${thumbnailProgress.status}`}>
                  <span className="scan-pulse" />
                  <div>
                    <strong>
                      {thumbnailProgress.status === "generating"
                        ? `正在生成 · ${number.format(thumbnailProgress.processed)} / ${number.format(thumbnailProgress.total)}`
                        : thumbnailProgress.status === "completed"
                          ? "本批预览已完成"
                          : "预览生成已停止"}
                    </strong>
                    <small>
                      成功 {number.format(thumbnailProgress.ready)} · 失败{" "}
                      {number.format(thumbnailProgress.failed)}
                    </small>
                  </div>
                </div>
              )}

              {thumbnailPreviews.length > 0 && (
                <div className="preview-grid">
                  {thumbnailPreviews.map((preview) => (
                    <figure key={preview.mediaId}>
                      <img
                        src={convertFileSrc(preview.cachePath)}
                        alt={new Date(preview.capturedAt).toLocaleDateString("zh-CN")}
                      />
                      {preview.mediaKind === "video" && <span className="video-badge">▶</span>}
                      <figcaption>
                        {new Date(preview.capturedAt).toLocaleDateString("zh-CN")}
                      </figcaption>
                    </figure>
                  ))}
                </div>
              )}
            </section>
          ) : null}
          </section>
        ) : index ? (
          <TimelineView totalMedia={index.total} onError={setError} />
        ) : null}
      </main>
    </div>
  );
}

export default App;
