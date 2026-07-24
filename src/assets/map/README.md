# Offline map data

## World

`ne_110m_admin_0_countries.json` contains Natural Earth 1:110m Admin 0
country boundaries in GeoJSON format.

- Source: https://github.com/nvkelso/natural-earth-vector/blob/master/geojson/ne_110m_admin_0_countries.geojson
- Retrieved: 2026-07-24
- Source SHA-256: `6866C877D39CBA9C357620878839B336D569F8C662D3CFAB4CB1DBE2D39C977F`
- Bundled geometry-only SHA-256: `B91612BB47C3B7C0D158602E06C57F5F8764E5EA9FBC0310B00D913B235A9AD3`
- License: public domain
- Terms: https://www.naturalearthdata.com/about/terms-of-use/

Unused country properties were removed mechanically; all polygon and multipolygon
coordinates are unchanged. The resulting file is bundled with the application and
is not fetched from the internet at runtime.

## China provinces and prefectures

`china-admin-2023.json` is the combined, quantized TopoJSON distribution from
`cn-atlas` 0.1.2. It contains 34 province-level regions and 372 prefecture-level
regions with six-digit division codes and Chinese names.

- Source: https://github.com/BarbarossaWang/cn-atlas
- Package: https://www.npmjs.com/package/cn-atlas/v/0.1.2
- SHA-256: `CEE4CF96AA903F996E14AB1F11B05E62C0B796FA29D4E5D376D9BDC82026C6AD`
- License: ISC

## China counties

`china-counties/*.json` contains 2,818 county-level regions split into 31
province chunks. Only chunks intersecting the current viewport are loaded.

- Source: https://github.com/zhChuXiao/ChinaGeoJson
- Source commit: `ad4d584bb975d7ab76fb9d22ae23ccdbfacef790`
- Upstream geometry: Alibaba DataV GeoAtlas
- Generated: 2026-07-24
- Aggregate manifest SHA-256: `A0E5FF7965188D61EB59162C10C6BCF5608EB3A4135412B3DA84BAD58F3C08C8`
- License: MIT

The source coordinates are GCJ-02. `scripts/build-china-map.mjs` converts them
to WGS-84 so they align with camera EXIF GPS, retains Chinese names and division
codes, simplifies rings at roughly 100-meter tolerance, and writes lazy-loaded
province chunks. Rebuild with:

```powershell
node .\scripts\build-china-map.mjs E:\path\to\ChinaGeoJson
```

See `THIRD_PARTY_NOTICES.md` for license notices.
