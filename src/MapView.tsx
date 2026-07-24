import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import MediaViewer from "./MediaViewer";
import type { TimelineItem } from "./media";
import { dateFromArchive } from "./media";
import "./MapView.css";

type TimelineMonth = {
  key: string;
  total: number;
  photos: number;
  videos: number;
  withLocation: number;
};

type MapOverview = {
  total: number;
  photos: number;
  videos: number;
  west: number | null;
  east: number | null;
  south: number | null;
  north: number | null;
  firstAt: string | null;
  lastAt: string | null;
};

type MapCluster = {
  cellX: number;
  cellY: number;
  latitude: number;
  longitude: number;
  total: number;
  photos: number;
  videos: number;
  firstAt: string;
  lastAt: string;
  representativeMediaId: number;
  west: number;
  east: number;
  south: number;
  north: number;
};

type MapClusterWindow = {
  total: number;
  items: TimelineItem[];
};

type MapViewProps = {
  totalLocated: number;
  onError: (message: string) => void;
};

type Point = { x: number; y: number };
type Center = { longitude: number; latitude: number };

const MIN_ZOOM = 2;
const MAX_ZOOM = 16;
const TILE_SIZE = 256;
const numberFormatter = new Intl.NumberFormat("zh-CN");
const monthFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "long",
});
const shortDateFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "short",
  day: "numeric",
});

// A deliberately low-detail, bundled geographic silhouette. It provides
// orientation without making any network request for tiles or location data.
const LANDMASSES: Array<Array<[number, number]>> = [
  [
    [-168, 70], [-150, 60], [-140, 58], [-130, 50], [-124, 40], [-117, 32],
    [-105, 24], [-95, 19], [-84, 22], [-80, 30], [-75, 39], [-65, 45],
    [-58, 52], [-64, 60], [-82, 68], [-105, 73], [-135, 72], [-168, 70],
  ],
  [
    [-82, 12], [-72, 10], [-62, 5], [-50, -3], [-42, -15], [-50, -28],
    [-57, -38], [-67, -52], [-74, -40], [-79, -20], [-82, 0], [-82, 12],
  ],
  [
    [-10, 36], [0, 44], [15, 48], [30, 46], [40, 40], [50, 44], [65, 52],
    [85, 55], [105, 52], [125, 48], [145, 58], [165, 60], [178, 52],
    [160, 42], [142, 36], [126, 30], [112, 20], [100, 8], [84, 8],
    [72, 20], [58, 26], [44, 30], [34, 34], [25, 38], [15, 36], [5, 35],
    [-10, 36],
  ],
  [
    [-17, 35], [2, 37], [18, 33], [32, 25], [42, 12], [48, -2],
    [40, -18], [30, -32], [18, -35], [7, -28], [-2, -10], [-10, 8],
    [-17, 22], [-17, 35],
  ],
  [
    [112, -11], [130, -12], [145, -20], [153, -30], [146, -40],
    [132, -43], [117, -34], [112, -22], [112, -11],
  ],
  [
    [-52, 60], [-42, 72], [-26, 80], [-18, 72], [-30, 62], [-52, 60],
  ],
  [[130, 32], [136, 35], [142, 44], [146, 42], [141, 34], [130, 32]],
  [[47, -13], [50, -17], [48, -25], [44, -20], [47, -13]],
];

function worldSize(zoom: number) {
  return TILE_SIZE * 2 ** zoom;
}

function project(longitude: number, latitude: number, zoom: number): Point {
  const size = worldSize(zoom);
  const safeLatitude = Math.max(-85.05112878, Math.min(85.05112878, latitude));
  const sin = Math.sin((safeLatitude * Math.PI) / 180);
  return {
    x: ((longitude + 180) / 360) * size,
    y: (0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * size,
  };
}

function unproject(x: number, y: number, zoom: number): Center {
  const size = worldSize(zoom);
  const longitude = (x / size) * 360 - 180;
  const n = Math.PI - (2 * Math.PI * y) / size;
  const latitude = (180 / Math.PI) * Math.atan(Math.sinh(n));
  return {
    longitude: Math.max(-180, Math.min(180, longitude)),
    latitude: Math.max(-85, Math.min(85, latitude)),
  };
}

function formatMonth(key: string) {
  return monthFormatter.format(new Date(`${key}-01T12:00:00`));
}

function overviewCenter(overview: MapOverview): Center {
  return {
    longitude: ((overview.west ?? 0) + (overview.east ?? 0)) / 2,
    latitude: ((overview.south ?? 0) + (overview.north ?? 0)) / 2,
  };
}

function fitZoom(overview: MapOverview, width: number, height: number) {
  if (
    overview.west == null ||
    overview.east == null ||
    overview.south == null ||
    overview.north == null
  ) {
    return MIN_ZOOM;
  }
  for (let zoom = MAX_ZOOM; zoom >= MIN_ZOOM; zoom -= 1) {
    const northWest = project(overview.west, overview.north, zoom);
    const southEast = project(overview.east, overview.south, zoom);
    if (
      Math.abs(southEast.x - northWest.x) <= width * 0.72 &&
      Math.abs(southEast.y - northWest.y) <= height * 0.68
    ) {
      return zoom;
    }
  }
  return MIN_ZOOM;
}

export default function MapView({ totalLocated, onError }: MapViewProps) {
  const mapRef = useRef<HTMLDivElement>(null);
  const clusterRequest = useRef(0);
  const dragState = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    centerWorld: Point;
  } | null>(null);
  const [months, setMonths] = useState<TimelineMonth[]>([]);
  const [selectedMonth, setSelectedMonth] = useState<string | null>(null);
  const [overview, setOverview] = useState<MapOverview | null>(null);
  const [clusters, setClusters] = useState<MapCluster[]>([]);
  const [center, setCenter] = useState<Center>({ longitude: 105, latitude: 35 });
  const [zoom, setZoom] = useState(3);
  const [viewport, setViewport] = useState({ width: 800, height: 600 });
  const [loading, setLoading] = useState(true);
  const [dragging, setDragging] = useState(false);
  const [selectedCluster, setSelectedCluster] = useState<MapCluster | null>(null);
  const [clusterWindow, setClusterWindow] = useState<MapClusterWindow | null>(null);
  const [clusterLoading, setClusterLoading] = useState(false);
  const [viewer, setViewer] = useState<TimelineItem | null>(null);
  const [failedImages, setFailedImages] = useState<Set<number>>(() => new Set());
  const [generating, setGenerating] = useState(false);

  useEffect(() => {
    invoke<TimelineMonth[]>("timeline_months")
      .then((result) => setMonths(result.filter((month) => month.withLocation > 0)))
      .catch((reason) => onError(String(reason)));
  }, [onError]);

  useLayoutEffect(() => {
    const element = mapRef.current;
    if (!element) return;
    const update = () =>
      setViewport({
        width: Math.max(320, element.clientWidth),
        height: Math.max(320, element.clientHeight),
      });
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setSelectedCluster(null);
    setClusterWindow(null);
    invoke<MapOverview>("map_overview", { month: selectedMonth })
      .then((result) => {
        if (!active) return;
        setOverview(result);
        if (result.total > 0) {
          setCenter(overviewCenter(result));
          setZoom(fitZoom(result, viewport.width, viewport.height));
        }
      })
      .catch((reason) => onError(String(reason)))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [onError, selectedMonth, viewport.height, viewport.width]);

  const centerWorld = useMemo(
    () => project(center.longitude, center.latitude, zoom),
    [center, zoom],
  );
  const bounds = useMemo(() => {
    const northWest = unproject(
      Math.max(0, centerWorld.x - viewport.width / 2),
      Math.max(0, centerWorld.y - viewport.height / 2),
      zoom,
    );
    const southEast = unproject(
      Math.min(worldSize(zoom), centerWorld.x + viewport.width / 2),
      Math.min(worldSize(zoom), centerWorld.y + viewport.height / 2),
      zoom,
    );
    return {
      west: northWest.longitude,
      east: southEast.longitude,
      north: northWest.latitude,
      south: southEast.latitude,
    };
  }, [centerWorld, viewport, zoom]);

  useEffect(() => {
    if (!overview?.total) {
      setClusters([]);
      return;
    }
    const sequence = ++clusterRequest.current;
    const timer = window.setTimeout(() => {
      invoke<MapCluster[]>("map_clusters", {
        ...bounds,
        zoom,
        month: selectedMonth,
      })
        .then((result) => {
          if (sequence === clusterRequest.current) setClusters(result);
        })
        .catch((reason) => onError(String(reason)));
    }, 90);
    return () => window.clearTimeout(timer);
  }, [bounds, onError, overview?.total, selectedMonth, zoom]);

  const visibleClusters = useMemo(
    () =>
      clusters
        .map((cluster) => {
          const point = project(cluster.longitude, cluster.latitude, zoom);
          return {
            cluster,
            x: point.x - centerWorld.x + viewport.width / 2,
            y: point.y - centerWorld.y + viewport.height / 2,
          };
        })
        .filter(
          (entry) =>
            entry.x > -80 &&
            entry.x < viewport.width + 80 &&
            entry.y > -80 &&
            entry.y < viewport.height + 80,
        ),
    [centerWorld, clusters, viewport, zoom],
  );

  const landPaths = useMemo(
    () =>
      LANDMASSES.map((polygon) =>
        polygon
          .map(([longitude, latitude], index) => {
            const point = project(longitude, latitude, zoom);
            const x = point.x - centerWorld.x + viewport.width / 2;
            const y = point.y - centerWorld.y + viewport.height / 2;
            return `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
          })
          .join(" "),
      ),
    [centerWorld, viewport, zoom],
  );

  const gridStep = zoom <= 2 ? 60 : zoom <= 4 ? 20 : zoom <= 6 ? 5 : zoom <= 9 ? 1 : 0.2;
  const longitudeLines = useMemo(() => {
    const lines: number[] = [];
    const first = Math.ceil(bounds.west / gridStep) * gridStep;
    for (let value = first; value <= bounds.east && lines.length < 50; value += gridStep) {
      lines.push(value);
    }
    return lines;
  }, [bounds.east, bounds.west, gridStep]);
  const latitudeLines = useMemo(() => {
    const lines: number[] = [];
    const first = Math.ceil(bounds.south / gridStep) * gridStep;
    for (let value = first; value <= bounds.north && lines.length < 50; value += gridStep) {
      lines.push(value);
    }
    return lines;
  }, [bounds.north, bounds.south, gridStep]);

  function changeZoom(nextZoom: number) {
    setZoom(Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, nextZoom)));
    setSelectedCluster(null);
    setClusterWindow(null);
  }

  function selectMonth(month: string | null) {
    if (month === selectedMonth) return;
    clusterRequest.current += 1;
    setClusters([]);
    setSelectedCluster(null);
    setClusterWindow(null);
    setSelectedMonth(month);
  }

  function fitCurrentOverview() {
    if (!overview?.total) return;
    setCenter(overviewCenter(overview));
    setZoom(fitZoom(overview, viewport.width, viewport.height));
    setSelectedCluster(null);
    setClusterWindow(null);
  }

  function handlePointerDown(event: React.PointerEvent<HTMLDivElement>) {
    const element = event.currentTarget;
    element.setPointerCapture(event.pointerId);
    dragState.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      centerWorld,
    };
    setDragging(true);
  }

  function handlePointerMove(event: React.PointerEvent<HTMLDivElement>) {
    const drag = dragState.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const size = worldSize(zoom);
    const nextX = Math.max(0, Math.min(size, drag.centerWorld.x - (event.clientX - drag.startX)));
    const nextY = Math.max(0, Math.min(size, drag.centerWorld.y - (event.clientY - drag.startY)));
    setCenter(unproject(nextX, nextY, zoom));
  }

  function handlePointerUp(event: React.PointerEvent<HTMLDivElement>) {
    if (dragState.current?.pointerId === event.pointerId) {
      dragState.current = null;
      setDragging(false);
    }
  }

  async function selectCluster(cluster: MapCluster) {
    setSelectedCluster(cluster);
    setClusterLoading(true);
    setFailedImages(new Set());
    try {
      setClusterWindow(
        await invoke<MapClusterWindow>("map_cluster_items", {
          west: cluster.west,
          east: cluster.east,
          south: cluster.south,
          north: cluster.north,
          month: selectedMonth,
          limit: 40,
        }),
      );
    } catch (reason) {
      onError(String(reason));
    } finally {
      setClusterLoading(false);
    }
  }

  async function openViewer(mediaId: number) {
    try {
      setViewer(await invoke<TimelineItem>("open_timeline_media", { mediaId }));
    } catch (reason) {
      onError(String(reason));
    }
  }

  async function generateRegionPreviews() {
    if (!selectedCluster || !clusterWindow) return;
    const mediaIds = clusterWindow.items
      .filter((item) => !item.thumbnailPath)
      .slice(0, 24)
      .map((item) => item.id);
    if (!mediaIds.length) return;
    setGenerating(true);
    try {
      await invoke("generate_timeline_thumbnails", { mediaIds });
      await selectCluster(selectedCluster);
    } catch (reason) {
      onError(String(reason));
    } finally {
      setGenerating(false);
    }
  }

  function screenPoint(longitude: number, latitude: number) {
    const point = project(longitude, latitude, zoom);
    return {
      x: point.x - centerWorld.x + viewport.width / 2,
      y: point.y - centerWorld.y + viewport.height / 2,
    };
  }

  return (
    <>
      <section className="map-shell">
        <aside className="map-time-rail" aria-label="地图时间筛选">
          <div className="map-rail-heading">
            <span className="section-label">TIME FILTER</span>
            <strong>{numberFormatter.format(totalLocated)} 个坐标</strong>
          </div>
          <div className="map-month-list">
            <button
              className={selectedMonth == null ? "active" : ""}
              type="button"
              onClick={() => selectMonth(null)}
            >
              <span>全部时间</span>
              <small>{numberFormatter.format(totalLocated)}</small>
            </button>
            {months.map((month) => (
              <button
                className={selectedMonth === month.key ? "active" : ""}
                key={month.key}
                type="button"
                onClick={() => selectMonth(month.key)}
              >
                <span>{formatMonth(month.key)}</span>
                <small>{numberFormatter.format(month.withLocation)}</small>
              </button>
            ))}
          </div>
          <div className="offline-note">
            <span>◎</span>
            <div>
              <strong>离线坐标底图</strong>
              <small>不请求在线地图服务</small>
            </div>
          </div>
        </aside>

        <div className="map-stage">
          <header className="map-heading">
            <div>
              <span className="section-label">SPATIAL MEMORY</span>
              <h2>{selectedMonth ? formatMonth(selectedMonth) : "所有地点"}</h2>
              <p>
                {overview
                  ? `${numberFormatter.format(overview.photos)} 张照片 · ${numberFormatter.format(overview.videos)} 段视频`
                  : "正在读取坐标…"}
              </p>
            </div>
            <div className="map-heading-stats">
              <div>
                <span>当前筛选</span>
                <strong>{numberFormatter.format(overview?.total ?? 0)}</strong>
              </div>
              <div>
                <span>可见聚合</span>
                <strong>{numberFormatter.format(clusters.length)}</strong>
              </div>
            </div>
          </header>

          <div
            className={`geo-map ${dragging ? "dragging" : ""}`}
            ref={mapRef}
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={handlePointerUp}
            onPointerCancel={handlePointerUp}
            onWheel={(event) => {
              event.preventDefault();
              changeZoom(zoom + (event.deltaY < 0 ? 1 : -1));
            }}
          >
            <svg
              className="map-geography"
              viewBox={`0 0 ${viewport.width} ${viewport.height}`}
              aria-hidden="true"
            >
              <rect width={viewport.width} height={viewport.height} className="map-ocean" />
              {longitudeLines.map((longitude) => {
                const start = screenPoint(longitude, bounds.north);
                const end = screenPoint(longitude, bounds.south);
                return (
                  <line
                    className="map-grid-line"
                    key={`lon-${longitude}`}
                    x1={start.x}
                    x2={end.x}
                    y1={start.y}
                    y2={end.y}
                  />
                );
              })}
              {latitudeLines.map((latitude) => {
                const start = screenPoint(bounds.west, latitude);
                const end = screenPoint(bounds.east, latitude);
                return (
                  <line
                    className="map-grid-line"
                    key={`lat-${latitude}`}
                    x1={start.x}
                    x2={end.x}
                    y1={start.y}
                    y2={end.y}
                  />
                );
              })}
              {landPaths.map((path, index) => (
                <path className="map-land" d={`${path} Z`} key={index} />
              ))}
            </svg>

            <div className="map-coordinate">
              {center.latitude.toFixed(3)}, {center.longitude.toFixed(3)} · Z{zoom}
            </div>

            <div className="map-controls" onPointerDown={(event) => event.stopPropagation()}>
              <button type="button" aria-label="放大地图" onClick={() => changeZoom(zoom + 1)}>
                +
              </button>
              <button type="button" aria-label="缩小地图" onClick={() => changeZoom(zoom - 1)}>
                −
              </button>
              <button type="button" aria-label="适配所有点位" onClick={fitCurrentOverview}>
                ⌂
              </button>
            </div>

            {visibleClusters.map(({ cluster, x, y }) => {
              const diameter = Math.min(62, 24 + Math.log2(cluster.total + 1) * 5);
              return (
                <button
                  className={`map-marker ${
                    selectedCluster?.cellX === cluster.cellX &&
                    selectedCluster?.cellY === cluster.cellY
                      ? "active"
                      : ""
                  }`}
                  key={`${cluster.cellX}-${cluster.cellY}`}
                  type="button"
                  style={{
                    width: diameter,
                    height: diameter,
                    left: x,
                    top: y,
                  }}
                  title={`${cluster.total} 个媒体`}
                  onPointerDown={(event) => event.stopPropagation()}
                  onClick={() => selectCluster(cluster)}
                  onDoubleClick={() => {
                    setCenter({
                      longitude: cluster.longitude,
                      latitude: cluster.latitude,
                    });
                    changeZoom(zoom + 2);
                  }}
                >
                  {cluster.total > 1 ? numberFormatter.format(cluster.total) : "•"}
                </button>
              );
            })}

            {loading && (
              <div className="map-loading">
                <span className="scan-pulse" />
                正在聚合坐标…
              </div>
            )}

            {overview?.total === 0 && !loading && (
              <div className="map-empty">这个时间范围内没有 GPS 信息</div>
            )}

            {selectedCluster && (
              <section
                className="map-cluster-drawer"
                onPointerDown={(event) => event.stopPropagation()}
              >
                <header>
                  <div>
                    <span className="section-label">LOCATION CLUSTER</span>
                    <h3>
                      {selectedCluster.latitude.toFixed(4)},{" "}
                      {selectedCluster.longitude.toFixed(4)}
                    </h3>
                    <p>
                      {shortDateFormatter.format(dateFromArchive(selectedCluster.firstAt))} —{" "}
                      {shortDateFormatter.format(dateFromArchive(selectedCluster.lastAt))}
                    </p>
                  </div>
                  <div className="cluster-actions">
                    {clusterWindow?.items.some((item) => !item.thumbnailPath) && (
                      <button
                        className="text-button"
                        type="button"
                        disabled={generating}
                        onClick={generateRegionPreviews}
                      >
                        {generating ? "正在生成…" : "补全区域预览"}
                      </button>
                    )}
                    <button
                      className="cluster-close"
                      type="button"
                      aria-label="关闭区域媒体"
                      onClick={() => {
                        setSelectedCluster(null);
                        setClusterWindow(null);
                      }}
                    >
                      ×
                    </button>
                  </div>
                </header>
                {clusterLoading ? (
                  <div className="cluster-loading">正在读取这个地点…</div>
                ) : (
                  <>
                    <div className="cluster-media-strip">
                      {(clusterWindow?.items ?? []).map((item) => (
                        <button
                          className="cluster-media"
                          key={item.id}
                          type="button"
                          onClick={() => openViewer(item.id)}
                        >
                          {item.thumbnailPath && !failedImages.has(item.id) ? (
                            <img
                              src={convertFileSrc(item.thumbnailPath)}
                              alt=""
                              onError={() =>
                                setFailedImages((current) => new Set(current).add(item.id))
                              }
                            />
                          ) : (
                            <span>{item.mediaKind === "video" ? "▶" : "◇"}</span>
                          )}
                          <small>
                            {new Intl.DateTimeFormat("zh-CN", {
                              month: "short",
                              day: "numeric",
                            }).format(dateFromArchive(item.capturedAt))}
                          </small>
                        </button>
                      ))}
                    </div>
                    <footer>
                      显示 {numberFormatter.format(clusterWindow?.items.length ?? 0)} /{" "}
                      {numberFormatter.format(clusterWindow?.total ?? selectedCluster.total)} 项
                    </footer>
                  </>
                )}
              </section>
            )}
          </div>
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
