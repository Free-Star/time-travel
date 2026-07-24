export type TimelineItem = {
  id: number;
  path: string;
  relativePath: string;
  mediaKind: "photo" | "video";
  extension: string;
  sizeBytes: number;
  capturedAt: string;
  capturedSource: string;
  capturedPrecision: string;
  latitude: number | null;
  longitude: number | null;
  width: number | null;
  height: number | null;
  thumbnailPath: string | null;
};

export function dateFromArchive(value: string) {
  return new Date(value.includes("T") ? value : `${value}T12:00:00`);
}

export function formatMediaBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
