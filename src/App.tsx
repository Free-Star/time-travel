import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import "./App.css";
import MapView from "./MapView";
import TimelineView from "./TimelineView";
import JournalView, { type JournalSummary } from "./JournalView";
import SettingsView from "./SettingsView";

type LibrarySummary = {
  root: string;
  displayName: string;
  topLevelFolders: number;
  topLevelMedia: number;
  projectDirectoryExcluded: boolean;
  writePolicy: string;
  online: boolean;
  active: boolean;
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

type JournalScanReport = {
  discovered: number;
  indexed: number;
  unchanged: number;
  skipped: number;
  removed: number;
  summary: JournalSummary;
};

function App() {
  const [view, setView] = useState<"home" | "timeline" | "map" | "journal" | "settings">("home");
  const [library, setLibrary] = useState<LibrarySummary | null>(null);
  const [libraries, setLibraries] = useState<LibrarySummary[]>([]);
  const [index, setIndex] = useState<IndexSummary | null>(null);
  const [, setScanProgress] = useState<ScanProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [journal, setJournal] = useState<JournalSummary | null>(null);
  const [journalScanning, setJournalScanning] = useState(false);
  const [, setJournalReport] = useState<JournalScanReport | null>(null);
  const [, setBusy] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    Promise.all([
      invoke<LibrarySummary | null>("current_library"),
      invoke<LibrarySummary[]>("library_roots"),
    ])
      .then(([summary, roots]) => {
        setLibrary(summary);
        setLibraries(roots);
        if (summary?.online) return loadLibraryData();
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
    await Promise.all([
      loadIndex(),
      invoke<JournalSummary | null>("current_journal").then(setJournal),
    ]);
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
      setLibraries(await invoke<LibrarySummary[]>("library_roots"));
      await loadLibraryData();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function activateLibrary(root: string) {
    setError("");
    try {
      const summary = await invoke<LibrarySummary>("activate_library", { root });
      setLibrary(summary);
      setLibraries(await invoke<LibrarySummary[]>("library_roots"));
      setIndex(null);
      setJournal(null);
      await loadLibraryData();
      setView("home");
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function removeLibrary(root: string) {
    setError("");
    try {
      const summary = await invoke<LibrarySummary | null>("remove_library", { root });
      setLibrary(summary);
      setLibraries(await invoke<LibrarySummary[]>("library_roots"));
      setIndex(null);
      setJournal(null);
      if (summary?.online) await loadLibraryData();
    } catch (reason) {
      setError(String(reason));
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

  async function chooseJournal() {
    setError("");
    const selected = await open({
      directory: true,
      multiple: false,
      title: "选择 Obsidian 日记目录",
    });
    if (!selected) return;
    setJournalScanning(true);
    try {
      const configured = await invoke<JournalSummary>("configure_journal", { root: selected });
      setJournal(configured);
      const report = await invoke<JournalScanReport>("scan_journal");
      setJournal(report.summary);
      setJournalReport(report);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setJournalScanning(false);
    }
  }

  async function scanJournal() {
    setError("");
    setJournalScanning(true);
    try {
      const report = await invoke<JournalScanReport>("scan_journal");
      setJournal(report.summary);
      setJournalReport(report);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setJournalScanning(false);
    }
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
          <button
            className={`nav-item ${view === "journal" ? "active" : ""}`}
            type="button"
            disabled={!journal?.total}
            onClick={() => {
              setError("");
              setView("journal");
            }}
          >
            <span>☷</span>日记
          </button>
          <button
            className={`nav-item ${view === "settings" ? "active" : ""}`}
            type="button"
            onClick={() => {
              setError("");
              setView("settings");
            }}
          >
            <span>⚙</span>设置
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
            <h1>
              {view === "timeline"
                ? "沿着时间，重看记忆"
                : view === "map"
                  ? "记忆曾在哪里发生"
                  : view === "journal"
                    ? "让文字与影像再次相遇"
                    : view === "settings"
                      ? "管理你的本地记忆库"
                : "欢迎回到 TimeTravel"}
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
          <section className="home-dashboard">
            <div className="home-intro">
              <div>
                <span className="section-label">YOUR MEMORY, LOCAL FIRST</span>
                <h2>从时间、地点与文字<br />重新走进记忆。</h2>
                <p>照片是画面，日记是叙述。所有内容只在本机读取和整理。</p>
              </div>
              <div className="home-summary">
                <div><span>媒体</span><strong>{number.format(index?.total ?? 0)}</strong></div>
                <div><span>地点</span><strong>{number.format(index?.withLocation ?? 0)}</strong></div>
                <div><span>日记</span><strong>{number.format(journal?.total ?? 0)}</strong></div>
              </div>
            </div>

            {!library || !index?.total ? (
              <button className="home-setup-notice" type="button" onClick={() => setView("settings")}>
                <span>还没有可浏览的媒体</span>
                <strong>前往设置，选择相册目录并建立索引 →</strong>
              </button>
            ) : index.needsMetadataRefresh ? (
              <button className="home-setup-notice" type="button" onClick={() => setView("settings")}>
                <span>索引规则已经更新</span>
                <strong>前往设置刷新媒体索引 →</strong>
              </button>
            ) : null}

            {error && <button className="home-error" type="button" onClick={() => setError("")}>{error}<span>×</span></button>}

            <section className="home-entry-grid" aria-label="浏览记忆">
                <button
                  type="button"
                  disabled={!index?.total || index.needsMetadataRefresh}
                  onClick={() => {
                    setError("");
                    setView("timeline");
                  }}
                >
                  <span className="home-entry-icon">◷</span>
                  <div className="home-entry-copy">
                    <small>CHRONOLOGY</small>
                    <strong>沿时间线浏览</strong>
                    <p>{number.format(index?.total ?? 0)} 个媒体，按年月组织</p>
                  </div>
                  <span className="home-entry-arrow">→</span>
                </button>
                <button
                  type="button"
                  disabled={!index?.withLocation || index.needsMetadataRefresh}
                  onClick={() => {
                    setError("");
                    setView("map");
                  }}
                >
                  <span className="home-entry-icon">⌖</span>
                  <div className="home-entry-copy">
                    <small>PLACES</small>
                    <strong>在地图上查看</strong>
                    <p>{number.format(index?.withLocation ?? 0)} 个坐标，按地点聚合</p>
                  </div>
                  <span className="home-entry-arrow">→</span>
                </button>
                <button
                  type="button"
                  disabled={!journal?.total}
                  onClick={() => {
                    setError("");
                    setView("journal");
                  }}
                >
                  <span className="home-entry-icon">☷</span>
                  <div className="home-entry-copy">
                    <small>JOURNAL</small>
                    <strong>阅读 Obsidian 日记</strong>
                    <p>{number.format(journal?.total ?? 0)} 篇日记，联动当天照片</p>
                  </div>
                  <span className="home-entry-arrow">→</span>
                </button>
            </section>

            <footer className="home-footer-note">
              <span>本地运行</span><i /> <span>媒体只读</span><i /> <span>不上传个人数据</span>
            </footer>
            </section>
        ) : view === "timeline" && index ? (
          <TimelineView totalMedia={index.total} onError={setError} />
        ) : view === "map" && index ? (
          <MapView totalLocated={index.withLocation} onError={setError} />
        ) : view === "journal" && journal ? (
          <JournalView summary={journal} onError={setError} />
        ) : view === "settings" ? (
          <SettingsView
            library={library}
            libraries={libraries}
            index={index}
            journal={journal}
            scanning={scanning}
            journalScanning={journalScanning}
            onChooseLibrary={chooseLibrary}
            onActivateLibrary={activateLibrary}
            onRemoveLibrary={removeLibrary}
            onScanLibrary={startScan}
            onChooseJournal={chooseJournal}
            onScanJournal={scanJournal}
            onError={setError}
          />
        ) : null}
      </main>
    </div>
  );
}

export default App;
