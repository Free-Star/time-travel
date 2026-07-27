import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { JournalSummary } from "./JournalView";
import "./SettingsView.css";

type LibrarySummary = {
  root: string;
  displayName: string;
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
};

type ThumbnailStatus = {
  totalMedia: number;
  ready: number;
  failed: number;
  cacheBytes: number;
  ffmpegAvailable: boolean;
};

type SettingsViewProps = {
  library: LibrarySummary | null;
  libraries: LibrarySummary[];
  index: IndexSummary | null;
  journal: JournalSummary | null;
  scanning: boolean;
  journalScanning: boolean;
  onChooseLibrary: () => void;
  onActivateLibrary: (root: string) => void;
  onRemoveLibrary: (root: string) => void;
  onScanLibrary: () => void;
  onChooseJournal: () => void;
  onScanJournal: () => void;
  onError: (message: string) => void;
};

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export default function SettingsView({
  library,
  libraries,
  index,
  journal,
  scanning,
  journalScanning,
  onChooseLibrary,
  onActivateLibrary,
  onRemoveLibrary,
  onScanLibrary,
  onChooseJournal,
  onScanJournal,
  onError,
}: SettingsViewProps) {
  const [thumbnailStatus, setThumbnailStatus] = useState<ThumbnailStatus | null>(null);
  const [markerDisplay, setMarkerDisplay] = useState<"count" | "thumbnail">(() =>
    window.localStorage.getItem("time-album-map-marker-display") === "thumbnail" ? "thumbnail" : "count",
  );

  useEffect(() => {
    if (!library?.online) {
      setThumbnailStatus(null);
      return;
    }
    invoke<ThumbnailStatus | null>("thumbnail_status")
      .then(setThumbnailStatus)
      .catch((reason) => onError(String(reason)));
  }, [library, onError]);

  function updateMarkerDisplay(value: "count" | "thumbnail") {
    setMarkerDisplay(value);
    window.localStorage.setItem("time-album-map-marker-display", value);
  }

  async function clearCache() {
    if (!window.confirm("只清除 TimeTravel 生成的缩略图缓存，原始媒体不会受到影响。继续吗？")) return;
    try {
      setThumbnailStatus(await invoke<ThumbnailStatus>("clear_thumbnail_cache"));
    } catch (reason) {
      onError(String(reason));
    }
  }

  return (
    <section className="settings-shell">
      <div className="settings-column">
        <section className="settings-section">
          <header><span className="section-label">MEDIA LIBRARY</span><h2>相册目录</h2></header>
          <div className="library-list">
            {libraries.map((item) => (
              <div className={`library-item ${item.active ? "active" : ""}`} key={item.root}>
                <span className={`library-state ${item.online ? "online" : "offline"}`} />
                <div><strong>{item.displayName}</strong><code>{item.root}</code><small>{item.online ? (item.active ? "当前相册" : "可用") : "目录离线，索引已保留"}</small></div>
                {!item.active && <button type="button" disabled={!item.online} onClick={() => onActivateLibrary(item.root)}>切换</button>}
                <button className="library-remove" type="button" title="仅移出列表，不删除索引或媒体" onClick={() => onRemoveLibrary(item.root)}>×</button>
              </div>
            ))}
            {!libraries.length && <p className="library-empty">尚未添加相册目录</p>}
          </div>
          <button className="secondary-button add-library" type="button" onClick={onChooseLibrary}>＋ 添加相册目录</button>
          {library?.online && (
            <div className="settings-row compact-row">
              <p>{index ? `${index.total.toLocaleString("zh-CN")} 个媒体 · ${index.withLocation.toLocaleString("zh-CN")} 个带定位` : "尚未建立索引"}</p>
              <button className="primary-button compact" type="button" disabled={scanning} onClick={onScanLibrary}>{scanning ? "正在扫描…" : "增量扫描"}<span>→</span></button>
            </div>
          )}
          {library && !library.online && <p className="offline-tip">当前目录暂时不可用。重新连接移动硬盘或网络目录后即可切换，已有索引不会被删除。</p>}
        </section>

        <section className="settings-section">
          <header><span className="section-label">OBSIDIAN</span><h2>日记目录</h2></header>
          <div className="settings-row">
            <div><strong>{journal?.displayName ?? "尚未配置"}</strong><code>{journal?.journalRoot ?? "选择 Obsidian Daily Notes 目录"}</code></div>
            <button className="secondary-button" type="button" disabled={!library?.online} onClick={onChooseJournal}>{journal ? "更换目录" : "选择目录"}</button>
          </div>
          {journal && (
            <div className="settings-row compact-row">
              <p>{journal.total.toLocaleString("zh-CN")} 篇日记 · Vault 完全只读</p>
              <button className="primary-button compact" type="button" disabled={journalScanning} onClick={onScanJournal}>{journalScanning ? "正在索引…" : "更新索引"}<span>→</span></button>
            </div>
          )}
        </section>

        <section className="settings-section">
          <header><span className="section-label">MAP DISPLAY</span><h2>地图标记</h2></header>
          <div className="settings-choice">
            <button className={markerDisplay === "count" ? "active" : ""} type="button" onClick={() => updateMarkerDisplay("count")}><strong>媒体数量</strong><small>速度最快，适合大量坐标</small></button>
            <button className={markerDisplay === "thumbnail" ? "active" : ""} type="button" onClick={() => updateMarkerDisplay("thumbnail")}><strong>照片缩略图</strong><small>按当前可见地点实时生成</small></button>
          </div>
        </section>

        <section className="settings-section">
          <header><span className="section-label">CACHE</span><h2>缩略图缓存</h2></header>
          <div className="settings-row">
            <div>
              <strong>{thumbnailStatus ? `${formatBytes(thumbnailStatus.cacheBytes)} · ${thumbnailStatus.ready.toLocaleString("zh-CN")} 项` : library ? "正在统计…" : "配置相册后可用"}</strong>
              <p>320px 轻量预览 · 邻近内容后台预热 · FFmpeg {thumbnailStatus?.ffmpegAvailable ? "可用" : "不可用"}</p>
            </div>
            <button className="danger-button" type="button" disabled={!thumbnailStatus?.ready} onClick={clearCache}>清空缓存</button>
          </div>
        </section>

        <section className="settings-section about-section">
          <div><span className="section-label">ABOUT</span><h2>TimeTravel</h2><p>0.0.2-beta · freestar</p></div>
          <p>本地优先 · 媒体与 Obsidian Vault 只读 · 不上传个人数据</p>
        </section>
      </div>
    </section>
  );
}
