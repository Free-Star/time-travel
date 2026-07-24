import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { feature as topologyFeature } from "topojson-client";
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
type Position = [number, number];
type GeoJsonGeometry =
  | { type: "Polygon"; coordinates: Position[][] }
  | { type: "MultiPolygon"; coordinates: Position[][][] };
type GeoJsonFeatureCollection = {
  features: Array<{
    geometry: GeoJsonGeometry;
    properties?: Record<string, string>;
  }>;
};
type CountryPolygon = {
  key: string;
  rings: Position[][];
  west: number;
  east: number;
  south: number;
  north: number;
};
type AdminRegion = {
  key: string;
  name: string;
  code: string;
  polygons: Position[][][];
  label: Position;
  west: number;
  east: number;
  south: number;
  north: number;
};
type ChinaTopology = {
  objects: {
    provinces: unknown;
    prefectures: unknown;
  };
};

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
const COUNTY_DATA_MODULES = import.meta.glob<{
  default: GeoJsonFeatureCollection;
}>("./assets/map/china-counties/*.json");

// Natural Earth 1:110m country boundaries, bundled with the application.
// The map is parsed locally and never makes a runtime network request.
function countryPolygonsFromGeoJson(source: GeoJsonFeatureCollection) {
  return source.features.flatMap<CountryPolygon>((feature, featureIndex) => {
    const polygons =
      feature.geometry.type === "Polygon"
        ? [feature.geometry.coordinates]
        : feature.geometry.coordinates;
    return polygons.map((rings, polygonIndex) => {
      const points = rings.flat();
      const longitudes = points.map(([longitude]) => longitude);
      const latitudes = points.map(([, latitude]) => latitude);
      return {
        key: `${featureIndex}-${polygonIndex}`,
        rings,
        west: Math.min(...longitudes),
        east: Math.max(...longitudes),
        south: Math.min(...latitudes),
        north: Math.max(...latitudes),
      };
    });
  });
}

function geometryPolygons(geometry: GeoJsonGeometry) {
  return geometry.type === "Polygon" ? [geometry.coordinates] : geometry.coordinates;
}

function ringCentroid(ring: Position[]): { point: Position; area: number } {
  let area = 0;
  let longitude = 0;
  let latitude = 0;
  for (let index = 0; index < ring.length - 1; index += 1) {
    const [x1, y1] = ring[index];
    const [x2, y2] = ring[index + 1];
    const cross = x1 * y2 - x2 * y1;
    area += cross;
    longitude += (x1 + x2) * cross;
    latitude += (y1 + y2) * cross;
  }
  area /= 2;
  if (Math.abs(area) < 1e-8) {
    const point = ring[Math.floor(ring.length / 2)] ?? [0, 0];
    return { point, area: 0 };
  }
  return {
    point: [longitude / (6 * area), latitude / (6 * area)],
    area: Math.abs(area),
  };
}

function adminRegionsFromGeoJson(source: GeoJsonFeatureCollection) {
  return source.features.map<AdminRegion>((feature, featureIndex) => {
    const polygons = geometryPolygons(feature.geometry);
    const points = polygons.flat(2);
    const longitudes = points.map(([longitude]) => longitude);
    const latitudes = points.map(([, latitude]) => latitude);
    const label = polygons
      .map((polygon) => ringCentroid(polygon[0]))
      .sort((left, right) => right.area - left.area)[0]?.point ?? [0, 0];
    const properties = feature.properties ?? {};
    return {
      key: properties.code ?? properties.id ?? String(featureIndex),
      name: properties["地名"] ?? properties.name ?? "未命名区域",
      code: properties["区划码"] ?? properties.code ?? properties.id ?? "",
      polygons,
      label,
      west: Math.min(...longitudes),
      east: Math.max(...longitudes),
      south: Math.min(...latitudes),
      north: Math.max(...latitudes),
    };
  });
}

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
  const timelineRef = useRef<HTMLDivElement>(null);
  const clusterRequest = useRef(0);
  const countyRequests = useRef(new Set<string>());
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
  const [mapDataLoading, setMapDataLoading] = useState(true);
  const [countryPolygons, setCountryPolygons] = useState<CountryPolygon[]>([]);
  const [provinceRegions, setProvinceRegions] = useState<AdminRegion[]>([]);
  const [cityRegions, setCityRegions] = useState<AdminRegion[]>([]);
  const [countyRegionsByProvince, setCountyRegionsByProvince] = useState<
    Record<string, AdminRegion[]>
  >({});
  const [dragging, setDragging] = useState(false);
  const [selectedCluster, setSelectedCluster] = useState<MapCluster | null>(null);
  const [clusterWindow, setClusterWindow] = useState<MapClusterWindow | null>(null);
  const [clusterLoading, setClusterLoading] = useState(false);
  const [viewer, setViewer] = useState<TimelineItem | null>(null);
  const [failedImages, setFailedImages] = useState<Set<number>>(() => new Set());
  const [generating, setGenerating] = useState(false);
  const displayMonths = useMemo(() => [...months].reverse(), [months]);

  useEffect(() => {
    invoke<TimelineMonth[]>("timeline_months")
      .then((result) => setMonths(result.filter((month) => month.withLocation > 0)))
      .catch((reason) => onError(String(reason)));
  }, [onError]);

  useEffect(() => {
    let active = true;
    Promise.all([
      import("./assets/map/ne_110m_admin_0_countries.json"),
      import("./assets/map/china-admin-2023.json"),
    ])
      .then(([worldModule, chinaModule]) => {
        const topology = chinaModule.default as unknown as ChinaTopology;
        const convertTopology = topologyFeature as unknown as (
          source: ChinaTopology,
          object: unknown,
        ) => GeoJsonFeatureCollection;
        if (active) {
          setCountryPolygons(
            countryPolygonsFromGeoJson(
              worldModule.default as unknown as GeoJsonFeatureCollection,
            ),
          );
          setProvinceRegions(
            adminRegionsFromGeoJson(
              convertTopology(topology, topology.objects.provinces),
            ),
          );
          setCityRegions(
            adminRegionsFromGeoJson(
              convertTopology(topology, topology.objects.prefectures),
            ),
          );
        }
      })
      .catch((reason) => onError(`无法读取离线地图数据：${String(reason)}`))
      .finally(() => active && setMapDataLoading(false));
    return () => {
      active = false;
    };
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

  useEffect(() => {
    const element = mapRef.current;
    if (!element) return;
    const handleWheel = (event: WheelEvent) => {
      event.preventDefault();
      const rectangle = element.getBoundingClientRect();
      changeZoomAt(
        zoom + (event.deltaY < 0 ? 1 : -1),
        event.clientX - rectangle.left,
        event.clientY - rectangle.top,
      );
    };
    element.addEventListener("wheel", handleWheel, { passive: false });
    return () => element.removeEventListener("wheel", handleWheel);
  }, [centerWorld, viewport, zoom]);
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
    if (zoom < 8 || cityRegions.length === 0) return;
    const provinceCodes = new Set(
      cityRegions
        .filter(
          (region) =>
            region.east >= bounds.west &&
            region.west <= bounds.east &&
            region.north >= bounds.south &&
            region.south <= bounds.north,
        )
        .map((region) => `${region.code.slice(0, 2)}0000`),
    );
    for (const provinceCode of provinceCodes) {
      if (
        countyRegionsByProvince[provinceCode] ||
        countyRequests.current.has(provinceCode)
      ) {
        continue;
      }
      const loader =
        COUNTY_DATA_MODULES[
          `./assets/map/china-counties/${provinceCode}.json`
        ];
      if (!loader) continue;
      countyRequests.current.add(provinceCode);
      loader()
        .then((module) => {
          setCountyRegionsByProvince((current) => ({
            ...current,
            [provinceCode]: adminRegionsFromGeoJson(module.default),
          }));
        })
        .catch((reason) =>
          onError(`无法读取${provinceCode}县级地图：${String(reason)}`),
        )
        .finally(() => countyRequests.current.delete(provinceCode));
    }
  }, [bounds, cityRegions, countyRegionsByProvince, onError, zoom]);

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

  const countryPaths = useMemo(() => {
    const longitudePadding = Math.max(2, (bounds.east - bounds.west) * 0.08);
    const latitudePadding = Math.max(2, (bounds.north - bounds.south) * 0.08);
    return countryPolygons.filter(
      (polygon) =>
        polygon.east >= bounds.west - longitudePadding &&
        polygon.west <= bounds.east + longitudePadding &&
        polygon.north >= bounds.south - latitudePadding &&
        polygon.south <= bounds.north + latitudePadding,
    ).map((polygon) => ({
      key: polygon.key,
      path: polygon.rings
        .map(
          (ring) =>
            `${ring
              .map(([longitude, latitude], index) => {
                const point = project(longitude, latitude, zoom);
                const x = point.x - centerWorld.x + viewport.width / 2;
                const y = point.y - centerWorld.y + viewport.height / 2;
                return `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
              })
              .join(" ")} Z`,
        )
        .join(" "),
    }));
  }, [bounds, centerWorld, countryPolygons, viewport, zoom]);

  const administrativeLayers = useMemo(() => {
    const longitudePadding = Math.max(0.5, (bounds.east - bounds.west) * 0.04);
    const latitudePadding = Math.max(0.5, (bounds.north - bounds.south) * 0.04);
    const renderRegions = (regions: AdminRegion[]) =>
      regions
        .filter(
          (region) =>
            region.east >= bounds.west - longitudePadding &&
            region.west <= bounds.east + longitudePadding &&
            region.north >= bounds.south - latitudePadding &&
            region.south <= bounds.north + latitudePadding,
        )
        .map((region) => {
          const labelPoint = project(region.label[0], region.label[1], zoom);
          return {
            key: region.key,
            name: region.name,
            code: region.code,
            labelX: labelPoint.x - centerWorld.x + viewport.width / 2,
            labelY: labelPoint.y - centerWorld.y + viewport.height / 2,
            path: region.polygons
              .map((polygon) =>
                polygon
                  .map(
                    (ring) =>
                      `${ring
                        .map(([longitude, latitude], index) => {
                          const point = project(longitude, latitude, zoom);
                          const x = point.x - centerWorld.x + viewport.width / 2;
                          const y = point.y - centerWorld.y + viewport.height / 2;
                          return `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
                        })
                        .join(" ")} Z`,
                  )
                  .join(" "),
              )
              .join(" "),
          };
        });
    return {
      provinces: zoom >= 4 ? renderRegions(provinceRegions) : [],
      cities: zoom >= 6 ? renderRegions(cityRegions) : [],
      counties:
        zoom >= 8
          ? renderRegions(Object.values(countyRegionsByProvince).flat())
          : [],
    };
  }, [
    bounds,
    centerWorld,
    cityRegions,
    countyRegionsByProvince,
    provinceRegions,
    viewport,
    zoom,
  ]);

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

  useEffect(() => {
    if (!selectedMonth) return;
    const selected = timelineRef.current?.querySelector<HTMLElement>(
      `[data-month="${selectedMonth}"]`,
    );
    selected?.scrollIntoView({
      behavior: "smooth",
      block: "nearest",
      inline: "center",
    });
  }, [selectedMonth]);

  function changeZoomAt(
    nextZoom: number,
    screenX = viewport.width / 2,
    screenY = viewport.height / 2,
  ) {
    const normalizedZoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, nextZoom));
    if (normalizedZoom === zoom) return;
    const anchor = unproject(
      centerWorld.x + screenX - viewport.width / 2,
      centerWorld.y + screenY - viewport.height / 2,
      zoom,
    );
    const nextAnchorWorld = project(anchor.longitude, anchor.latitude, normalizedZoom);
    const size = worldSize(normalizedZoom);
    const nextCenterX = Math.max(
      0,
      Math.min(size, nextAnchorWorld.x - screenX + viewport.width / 2),
    );
    const nextCenterY = Math.max(
      0,
      Math.min(size, nextAnchorWorld.y - screenY + viewport.height / 2),
    );
    setCenter(unproject(nextCenterX, nextCenterY, normalizedZoom));
    setZoom(normalizedZoom);
    setSelectedCluster(null);
    setClusterWindow(null);
  }

  function changeZoom(nextZoom: number) {
    changeZoomAt(nextZoom);
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
              {countryPaths.map((country) => (
                <path
                  className="map-land"
                  d={country.path}
                  fillRule="evenodd"
                  key={country.key}
                />
              ))}
              {administrativeLayers.provinces.map((region) => (
                <path
                  className="map-admin-boundary province"
                  d={region.path}
                  fillRule="evenodd"
                  key={`province-${region.key}`}
                />
              ))}
              {administrativeLayers.cities.map((region) => (
                <path
                  className="map-admin-boundary city"
                  d={region.path}
                  fillRule="evenodd"
                  key={`city-${region.key}`}
                />
              ))}
              {administrativeLayers.counties.map((region) => (
                <path
                  className="map-admin-boundary county"
                  d={region.path}
                  fillRule="evenodd"
                  key={`county-${region.key}`}
                />
              ))}
              {zoom >= 4 &&
                zoom < 6 &&
                administrativeLayers.provinces.map((region) => (
                  <text
                    className="map-place-label province"
                    key={`province-label-${region.key}`}
                    x={region.labelX}
                    y={region.labelY}
                  >
                    {region.name}
                  </text>
                ))}
              {zoom >= 6 &&
                zoom < 9 &&
                administrativeLayers.cities.map((region) => (
                  <text
                    className="map-place-label city"
                    key={`city-label-${region.key}`}
                    x={region.labelX}
                    y={region.labelY}
                  >
                    {region.name}
                  </text>
                ))}
              {zoom >= 9 &&
                administrativeLayers.counties.map((region) => (
                  <text
                    className="map-place-label county"
                    key={`county-label-${region.key}`}
                    x={region.labelX}
                    y={region.labelY}
                  >
                    {region.name}
                  </text>
                ))}
            </svg>

            <div className="map-admin-level">
              {zoom >= 8
                ? "县级边界"
                : zoom >= 6
                  ? "市级边界"
                  : zoom >= 4
                    ? "省级边界"
                    : "世界国界"}
            </div>
            <div className="map-coordinate">
              {center.latitude.toFixed(3)}, {center.longitude.toFixed(3)} · Z{zoom}
            </div>
            <div className="map-source">Natural Earth + ChinaGeoJson · 离线 JSON</div>

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
                    changeZoom(zoom + 2);
                    setCenter({
                      longitude: cluster.longitude,
                      latitude: cluster.latitude,
                    });
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

            {mapDataLoading && !loading && (
              <div className="map-loading">
                <span className="scan-pulse" />
                正在读取离线 GeoJSON…
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

          <section className="map-time-dock" aria-label="地图时间轴">
            <div className="map-timeline-heading">
              <span className="section-label">TIME AXIS</span>
              <strong>{numberFormatter.format(totalLocated)} 个坐标</strong>
            </div>
            <button
              className={`timeline-all ${selectedMonth == null ? "active" : ""}`}
              type="button"
              onClick={() => selectMonth(null)}
            >
              <span>全部</span>
              <small>{numberFormatter.format(totalLocated)}</small>
            </button>
            <div className="map-month-track" ref={timelineRef}>
              {displayMonths.map((month) => (
                <button
                  className={selectedMonth === month.key ? "active" : ""}
                  data-month={month.key}
                  key={month.key}
                  type="button"
                  title={`${formatMonth(month.key)} · ${month.withLocation} 个坐标`}
                  onClick={() => selectMonth(month.key)}
                >
                  <i />
                  <span>{formatMonth(month.key)}</span>
                  <small>{numberFormatter.format(month.withLocation)}</small>
                </button>
              ))}
            </div>
            <div className="offline-note">
              <span>◎</span>
              <div>
                <strong>真实离线地图</strong>
                <small>省 Z4 · 市 Z6 · 县 Z8</small>
              </div>
            </div>
          </section>
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
