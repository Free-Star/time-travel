import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import MediaViewer from "./MediaViewer";
import type { TimelineItem } from "./media";
import "./JournalView.css";

export type JournalSummary = {
  journalRoot: string;
  vaultRoot: string;
  displayName: string;
  total: number;
  firstDate: string | null;
  lastDate: string | null;
  lastScanAt: string | null;
};

type JournalMonth = { key: string; total: number };
type JournalEntry = {
  id: number;
  entryDate: string;
  title: string;
  content: string;
  path: string;
  relativePath: string;
  attachments: string[];
};

type JournalViewProps = {
  summary: JournalSummary;
  onError: (message: string) => void;
};

const monthFormatter = new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "long" });
const dateFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "long",
  day: "numeric",
  weekday: "long",
});

function localDate(value: string) {
  return new Date(`${value}T12:00:00`);
}

function readableMarkdown(content: string) {
  return content.replace(/!\[\[([^\]|]+)(?:\|[^\]]+)?\]\]/g, "> 附件：$1");
}

export default function JournalView({ summary, onError }: JournalViewProps) {
  const [months, setMonths] = useState<JournalMonth[]>([]);
  const [selectedMonth, setSelectedMonth] = useState("");
  const [entries, setEntries] = useState<JournalEntry[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [media, setMedia] = useState<TimelineItem[]>([]);
  const [viewer, setViewer] = useState<TimelineItem | null>(null);
  const [loading, setLoading] = useState(true);

  const selected = useMemo(
    () => entries.find((entry) => entry.id === selectedId) ?? entries[0] ?? null,
    [entries, selectedId],
  );

  useEffect(() => {
    invoke<JournalMonth[]>("journal_months")
      .then((result) => {
        setMonths(result);
        setSelectedMonth((current) => current || result[0]?.key || "");
      })
      .catch((reason) => onError(String(reason)))
      .finally(() => setLoading(false));
  }, [onError]);

  useEffect(() => {
    if (!selectedMonth) return;
    setLoading(true);
    invoke<JournalEntry[]>("journal_entries", { month: selectedMonth })
      .then((result) => {
        setEntries(result);
        setSelectedId(result[0]?.id ?? null);
      })
      .catch((reason) => onError(String(reason)))
      .finally(() => setLoading(false));
  }, [onError, selectedMonth]);

  useEffect(() => {
    if (!selected) {
      setMedia([]);
      return;
    }
    let active = true;
    const load = () => invoke<TimelineItem[]>("journal_media_for_date", { date: selected.entryDate });
    load()
      .then((result) => {
        if (active) setMedia(result);
        const ids = result.filter((item) => !item.thumbnailPath).slice(0, 24).map((item) => item.id);
        if (ids.length) {
          void invoke("ensure_timeline_thumbnails", { mediaIds: ids })
            .then(load)
            .then((refreshed) => active && setMedia(refreshed));
        }
      })
      .catch((reason) => onError(String(reason)));
    return () => {
      active = false;
    };
  }, [onError, selected?.entryDate]);

  async function openMedia(id: number) {
    try {
      setViewer(await invoke<TimelineItem>("open_timeline_media", { mediaId: id }));
    } catch (reason) {
      onError(String(reason));
    }
  }

  return (
    <>
      <section className="journal-shell">
        <aside className="journal-months">
          <div className="journal-source">
            <span className="section-label">OBSIDIAN</span>
            <strong>{summary.displayName}</strong>
            <small>{summary.total} 篇日记</small>
          </div>
          <div className="journal-month-list">
            {months.map((month) => (
              <button
                className={month.key === selectedMonth ? "active" : ""}
                key={month.key}
                type="button"
                onClick={() => setSelectedMonth(month.key)}
              >
                <span>{monthFormatter.format(localDate(`${month.key}-01`))}</span>
                <small>{month.total}</small>
              </button>
            ))}
          </div>
        </aside>

        <aside className="journal-days">
          <header>
            <span>本月记录</span>
            <strong>{entries.length}</strong>
          </header>
          {entries.map((entry) => (
            <button
              className={entry.id === selected?.id ? "active" : ""}
              key={entry.id}
              type="button"
              onClick={() => setSelectedId(entry.id)}
            >
              <time>{entry.entryDate.slice(8, 10)}</time>
              <span>
                <strong>{entry.title}</strong>
                <small>{entry.relativePath}</small>
              </span>
            </button>
          ))}
        </aside>

        <article className="journal-reader">
          {loading ? (
            <div className="journal-empty">正在读取日记…</div>
          ) : selected ? (
            <>
              <header>
                <span className="section-label">DAILY MEMORY</span>
                <h2>{selected.title}</h2>
                <p>{dateFormatter.format(localDate(selected.entryDate))}</p>
                <button
                  className="open-obsidian-button"
                  type="button"
                  onClick={() => invoke("open_journal_in_obsidian", { path: selected.path }).catch((reason) => onError(String(reason)))}
                >
                  在 Obsidian 中打开
                </button>
              </header>
              {selected.attachments.length > 0 && (
                <div className="journal-attachments">
                  {selected.attachments.filter((path) => /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(path)).map((path) => (
                    <img key={path} src={convertFileSrc(path)} alt="日记附件" loading="lazy" />
                  ))}
                </div>
              )}
              <div className="journal-markdown">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{readableMarkdown(selected.content)}</ReactMarkdown>
              </div>
            </>
          ) : (
            <div className="journal-empty">这个月份没有可读取的日期日记</div>
          )}
        </article>

        <aside className="journal-memory">
          <header>
            <span className="section-label">SAME DAY</span>
            <strong>{media.length} 个媒体</strong>
          </header>
          <div className="journal-media-grid">
            {media.map((item) => (
              <button key={item.id} type="button" onClick={() => openMedia(item.id)}>
                {item.thumbnailPath ? (
                  <img src={convertFileSrc(item.thumbnailPath)} alt="" />
                ) : (
                  <span>{item.mediaKind === "video" ? "▶" : "◇"}</span>
                )}
              </button>
            ))}
          </div>
          {!media.length && selected && <p>这一天只有文字记录</p>}
        </aside>
      </section>
      {viewer && (
        <MediaViewer item={viewer} onChange={setViewer} onClose={() => setViewer(null)} onError={onError} />
      )}
    </>
  );
}
