<div align="center">
  <img src="src-tauri/icons/logo-source.png" width="128" alt="TimeTravel Logo">
  <h1>TimeTravel</h1>
  <p>从时间与地点重新浏览个人记忆的本地桌面相册。</p>
  <p><strong>本地优先 · 媒体只读 · 离线地图</strong></p>
</div>

> 当前版本：`0.0.1-beta`。目前主要面向 Windows 10/11。

## 功能

- 按年、月组织照片和视频的时间线。
- 根据 EXIF GPS 在离线地图上展示拍摄地点。
- 中国省、市、县三级行政边界和中文地名。
- 地图缩放以鼠标位置为中心，底部时间轴支持滚轮切换月份。
- 地图聚合点可切换“照片数量”或“缩略图”显示。
- 递归扫描相册，不要求固定的文件夹结构。
- 根据 EXIF、媒体元数据、文件名和目录推断拍摄时间。
- 增量索引，只重新处理发生变化的文件。
- 按可见区域实时生成缩略图，优先复用内嵌预览和 Windows Shell 缓存。
- 照片与视频只读查看、前后导航和媒体详情。

## 媒体安全

TimeTravel 的首要原则是不修改媒体原件。

- 不修改文件内容、EXIF、文件时间、名称或目录位置。
- 媒体文件只通过只读方式打开。
- 数据库、设置和缩略图缓存保存在相册目录之外。
- 后端发现写入目标位于相册根目录内时会立即拒绝。
- 扫描和缩略图测试会校验源文件没有发生变化。
- 地图数据完全离线，照片坐标不会发送到网络服务。

## 技术栈

| 层级 | 实现 |
| --- | --- |
| 桌面应用 | Tauri 2、Windows WebView2 |
| 前端 | React 19、TypeScript、Vite 7、原生 CSS |
| 后端 | Rust |
| 本地索引 | SQLite、rusqlite |
| 媒体元数据 | kamadak-exif、Rust image、Windows Shell、FFmpeg 备用方案 |
| 地图 | Natural Earth、ChinaGeoJson、TopoJSON/GeoJSON |
| 安装包 | NSIS |

## 实现结构

```text
只读媒体目录
    │
    ├── Rust 递归扫描、EXIF/GPS 和日期识别
    │
    ├── SQLite 增量索引
    │
    └── 系统预览 / 内嵌预览 / 解码生成缩略图
            │
            ▼
      应用数据与缓存目录
            │
            ▼
    React 时间线与离线地图
```

时间线采用按月聚合和窗口化读取，不会一次将全部媒体加载到内存。地图聚合由 SQLite 根据当前视野、缩放级别和月份计算；县级地图数据仅在需要时加载。

## 开发环境

需要：

- Windows 10/11
- Node.js（包含 Corepack）
- Rust stable
- Visual Studio Build Tools，安装“使用 C++ 的桌面开发”
- WebView2 Runtime

启动开发模式：

```powershell
.\start.cmd
```

`start.cmd` 会通过 Corepack 使用项目指定的 pnpm，不要求全局安装 pnpm。

前端检查：

```powershell
corepack pnpm build
```

Rust 测试：

```powershell
corepack pnpm test:rust
```

当前测试包含扫描只读安全、缩略图源文件保护、时间线查询、地图聚合和日期识别。

## 打包

```powershell
.\package.cmd
```

安装包默认输出到：

```text
E:\TimeAlbumBuild\time-album\release\bundle\nsis\
```

项目将 Rust 构建缓存放在较短的独立路径，以规避 Windows 中文深路径和大型构建文件可能引发的问题。源码仍保留在项目目录。

## 目录

```text
src/                    React 前端
src/assets/map/         离线地图数据
src-tauri/src/          Rust 后端
src-tauri/icons/        应用图标
scripts/                开发、测试与打包脚本
docs/                   开发计划
```

## 后续计划

- 待完善媒体页面：缺少时间、定位和读取失败。
- 用户时间与地点修正，只写入数据库。
- 多级缩略图和缓存容量管理。
- 数据库备份、恢复和版本迁移。
- 年份快速跳转、地点搜索和快捷键。
- 代码签名、自动更新和便携版。

## 作者

`freestar`
