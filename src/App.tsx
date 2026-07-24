import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import "./App.css";
import MapView from "./MapView";
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

function App() {
  const [view, setView] = useState<"home" | "timeline" | "map">("home");
  const [library, setLibrary] = useState<LibrarySummary | null>(null);
  const [index, setIndex] = useState<IndexSummary | null>(null);
  const [scanProgress, setScanProgress] = useState<ScanProgress | null>(null);
  const [scanning, setScanning] = useState(false);
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

  async function loadLibraryData() {
    await loadIndex();
  }

  async function loadIndex() {
    const summary = await invoke<IndexSummary | null>("current_index");
    setIndex(summary);
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

  const number = new Intl.NumberFormat("zh-CN");

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">时</span>
          <div>
            <strong>TimeTravel</strong>
            <small>by freestar</small>
          </div>
        </div>

        <nav aria-label="主导航">
          <button
            className={`nav-item ${view === "home" ? "active" : ""}`}
            type="button"
            onClick={() => setView("home")}
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
          <button
            className={`nav-item ${view === "map" ? "active" : ""}`}
            type="button"
            disabled={!index?.withLocation || index.needsMetadataRefresh}
            onClick={() => {
              setError("");
              setView("map");
            }}
          >
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
            <span className="eyebrow">阶段 5 · 时间与空间联动</span>
            <h1>
              {view === "timeline"
                ? "沿着时间，重看记忆"
                : view === "map"
                  ? "记忆曾在哪里发生"
                : library
                  ? "相册已连接"
                  : "连接你的相册库"}
            </h1>
          </div>
          {view !== "map" && <div className="readonly-pill">只读模式</div>}
        </header>

        {view !== "home" && error && (
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
                TimeTravel 只读取媒体和元数据。索引、缩略图与所有人工修正都会保存在相册目录之外。
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
            <section className="explore-card" aria-label="浏览相册">
              <div className="explore-heading">
                <span className="section-label">开始浏览</span>
                <h3>从时间或地点，重新走进记忆</h3>
              </div>
              <div className="explore-actions">
                <button
                  type="button"
                  disabled={index.needsMetadataRefresh}
                  onClick={() => {
                    setError("");
                    setView("timeline");
                  }}
                >
                  <span className="explore-icon">◷</span>
                  <div>
                    <strong>沿时间线浏览</strong>
                    <small>{number.format(index.total)} 个媒体 · 按年月组织</small>
                  </div>
                  <span className="explore-arrow">→</span>
                </button>
                <button
                  type="button"
                  disabled={!index.withLocation || index.needsMetadataRefresh}
                  onClick={() => {
                    setError("");
                    setView("map");
                  }}
                >
                  <span className="explore-icon">⌖</span>
                  <div>
                    <strong>在地图上查看</strong>
                    <small>{number.format(index.withLocation)} 个坐标 · 按地点聚合</small>
                  </div>
                  <span className="explore-arrow">→</span>
                </button>
              </div>
            </section>
          ) : null}
          </section>
        ) : view === "timeline" && index ? (
          <TimelineView totalMedia={index.total} onError={setError} />
        ) : view === "map" && index ? (
          <MapView totalLocated={index.withLocation} onError={setError} />
        ) : null}
      </main>
    </div>
  );
}

export default App;
