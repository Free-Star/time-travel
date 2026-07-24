import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import "./App.css";

type LibrarySummary = {
  root: string;
  displayName: string;
  topLevelFolders: number;
  topLevelMedia: number;
  projectDirectoryExcluded: boolean;
  writePolicy: string;
};

function App() {
  const [library, setLibrary] = useState<LibrarySummary | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    invoke<LibrarySummary | null>("current_library")
      .then(setLibrary)
      .catch((reason) => setError(String(reason)))
      .finally(() => setBusy(false));
  }, []);

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
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

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
          <button className="nav-item active" type="button">
            <span>⌁</span>开始
          </button>
          <button className="nav-item" type="button" disabled>
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
            <span className="eyebrow">阶段 1 · 安全初始化</span>
            <h1>{library ? "相册已连接" : "连接你的相册库"}</h1>
          </div>
          <div className="readonly-pill">只读模式</div>
        </header>

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
                完整媒体扫描将在阶段 2 启用；当前只检查一级目录和安全边界。
              </p>
            </section>
          )}
        </section>
      </main>
    </div>
  );
}

export default App;
