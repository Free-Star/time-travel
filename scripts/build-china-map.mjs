import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const sourceRoot = process.argv[2];
if (!sourceRoot) {
  throw new Error("Usage: node scripts/build-china-map.mjs <ChinaGeoJson source directory>");
}

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const outputDirectory = path.resolve(
  scriptDirectory,
  "../src/assets/map/china-counties",
);

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function readFeatures(directory) {
  return fs
    .readdirSync(directory)
    .filter((name) => name.endsWith(".json"))
    .sort()
    .flatMap((name) => readJson(path.join(directory, name)).features ?? []);
}

function outsideChina(longitude, latitude) {
  return longitude < 72.004 || longitude > 137.8347 || latitude < 0.8293 || latitude > 55.8271;
}

function latitudeOffset(longitude, latitude) {
  let result =
    -100 +
    2 * longitude +
    3 * latitude +
    0.2 * latitude * latitude +
    0.1 * longitude * latitude +
    0.2 * Math.sqrt(Math.abs(longitude));
  result +=
    ((20 * Math.sin(6 * longitude * Math.PI) +
      20 * Math.sin(2 * longitude * Math.PI)) *
      2) /
    3;
  result +=
    ((20 * Math.sin(latitude * Math.PI) +
      40 * Math.sin((latitude / 3) * Math.PI)) *
      2) /
    3;
  result +=
    ((160 * Math.sin((latitude / 12) * Math.PI) +
      320 * Math.sin((latitude * Math.PI) / 30)) *
      2) /
    3;
  return result;
}

function longitudeOffset(longitude, latitude) {
  let result =
    300 +
    longitude +
    2 * latitude +
    0.1 * longitude * longitude +
    0.1 * longitude * latitude +
    0.1 * Math.sqrt(Math.abs(longitude));
  result +=
    ((20 * Math.sin(6 * longitude * Math.PI) +
      20 * Math.sin(2 * longitude * Math.PI)) *
      2) /
    3;
  result +=
    ((20 * Math.sin(longitude * Math.PI) +
      40 * Math.sin((longitude / 3) * Math.PI)) *
      2) /
    3;
  result +=
    ((150 * Math.sin((longitude / 12) * Math.PI) +
      300 * Math.sin((longitude / 30) * Math.PI)) *
      2) /
    3;
  return result;
}

function wgs84ToGcj02(longitude, latitude) {
  if (outsideChina(longitude, latitude)) return [longitude, latitude];
  const semiMajorAxis = 6378245;
  const eccentricitySquared = 0.006693421622965943;
  let deltaLatitude = latitudeOffset(longitude - 105, latitude - 35);
  let deltaLongitude = longitudeOffset(longitude - 105, latitude - 35);
  const radians = (latitude / 180) * Math.PI;
  let magic = Math.sin(radians);
  magic = 1 - eccentricitySquared * magic * magic;
  const rootMagic = Math.sqrt(magic);
  deltaLatitude =
    (deltaLatitude * 180) /
    (((semiMajorAxis * (1 - eccentricitySquared)) / (magic * rootMagic)) * Math.PI);
  deltaLongitude =
    (deltaLongitude * 180) /
    ((semiMajorAxis / rootMagic) * Math.cos(radians) * Math.PI);
  return [longitude + deltaLongitude, latitude + deltaLatitude];
}

function gcj02ToWgs84(longitude, latitude) {
  if (outsideChina(longitude, latitude)) return [longitude, latitude];
  let resultLongitude = longitude;
  let resultLatitude = latitude;
  for (let index = 0; index < 3; index += 1) {
    const [convertedLongitude, convertedLatitude] = wgs84ToGcj02(
      resultLongitude,
      resultLatitude,
    );
    resultLongitude -= convertedLongitude - longitude;
    resultLatitude -= convertedLatitude - latitude;
  }
  return [resultLongitude, resultLatitude];
}

function squaredSegmentDistance(point, start, end) {
  let x = start[0];
  let y = start[1];
  let deltaX = end[0] - x;
  let deltaY = end[1] - y;
  if (deltaX !== 0 || deltaY !== 0) {
    const progress =
      ((point[0] - x) * deltaX + (point[1] - y) * deltaY) /
      (deltaX * deltaX + deltaY * deltaY);
    if (progress > 1) {
      x = end[0];
      y = end[1];
    } else if (progress > 0) {
      x += deltaX * progress;
      y += deltaY * progress;
    }
  }
  deltaX = point[0] - x;
  deltaY = point[1] - y;
  return deltaX * deltaX + deltaY * deltaY;
}

function simplifySegment(points, first, last, toleranceSquared, simplified) {
  let furthestDistance = toleranceSquared;
  let furthestIndex = 0;
  for (let index = first + 1; index < last; index += 1) {
    const distance = squaredSegmentDistance(points[index], points[first], points[last]);
    if (distance > furthestDistance) {
      furthestIndex = index;
      furthestDistance = distance;
    }
  }
  if (furthestDistance > toleranceSquared) {
    if (furthestIndex - first > 1) {
      simplifySegment(points, first, furthestIndex, toleranceSquared, simplified);
    }
    simplified.push(points[furthestIndex]);
    if (last - furthestIndex > 1) {
      simplifySegment(points, furthestIndex, last, toleranceSquared, simplified);
    }
  }
}

function simplifyRing(ring, tolerance = 0.001) {
  if (ring.length <= 5) return ring;
  const open = ring.slice(0, -1);
  const simplified = [open[0]];
  simplifySegment(open, 0, open.length - 1, tolerance * tolerance, simplified);
  simplified.push(open[open.length - 1]);
  if (simplified.length < 3) return ring;
  simplified.push(simplified[0]);
  return simplified;
}

function transformGeometry(geometry) {
  const transformPoint = ([longitude, latitude]) =>
    gcj02ToWgs84(longitude, latitude).map((value) => Number(value.toFixed(6)));
  const polygons =
    geometry.type === "Polygon" ? [geometry.coordinates] : geometry.coordinates;
  const transformed = polygons.map((polygon) =>
    polygon.map((ring) => simplifyRing(ring.map(transformPoint))),
  );
  return {
    type: geometry.type,
    coordinates: geometry.type === "Polygon" ? transformed[0] : transformed,
  };
}

function normalizeFeature(feature) {
  const properties = feature.properties ?? {};
  const code = String(properties.adcode ?? properties.code ?? properties.id ?? "");
  const transformPoint = (point) =>
    Array.isArray(point) && point.length >= 2
      ? gcj02ToWgs84(point[0], point[1]).map((value) => Number(value.toFixed(6)))
      : undefined;
  return {
    type: "Feature",
    properties: {
      name: String(properties.name ?? code),
      code,
      level: String(properties.level ?? ""),
      center: transformPoint(properties.center),
      centroid: transformPoint(properties.centroid),
    },
    geometry: transformGeometry(feature.geometry),
  };
}

function uniqueFeatures(features) {
  const unique = new Map();
  for (const feature of features) {
    const normalized = normalizeFeature(feature);
    if (normalized.properties.code) {
      unique.set(normalized.properties.code, normalized);
    }
  }
  return [...unique.values()];
}

const counties = uniqueFeatures(readFeatures(path.join(sourceRoot, "citys")));
const collection = (features) => ({ type: "FeatureCollection", features });
const countiesByProvince = new Map();
for (const county of counties) {
  const provinceCode = `${county.properties.code.slice(0, 2)}0000`;
  const group = countiesByProvince.get(provinceCode) ?? [];
  group.push(county);
  countiesByProvince.set(provinceCode, group);
}
fs.mkdirSync(outputDirectory, { recursive: true });
for (const name of fs.readdirSync(outputDirectory)) {
  if (name.endsWith(".json")) {
    fs.unlinkSync(path.join(outputDirectory, name));
  }
}
for (const [provinceCode, features] of countiesByProvince) {
  fs.writeFileSync(
    path.join(outputDirectory, `${provinceCode}.json`),
    JSON.stringify(collection(features)),
  );
}

console.log(
  JSON.stringify({
    outputDirectory,
    bytes: fs
      .readdirSync(outputDirectory)
      .map((name) => fs.statSync(path.join(outputDirectory, name)).size)
      .reduce((sum, size) => sum + size, 0),
    provinceFiles: countiesByProvince.size,
    counties: counties.length,
  }),
);
