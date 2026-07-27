<div align="center">
  <img src="src-tauri/icons/logo-source.png" width="128" alt="TimeTravel Logo">
  <h1>TimeTravel</h1>
  <p>从时间与地点重新浏览个人记忆的本地桌面相册。</p>
  <p><strong>本地优先 · 媒体只读 · 离线地图</strong></p>
</div>

> 当前版本：`0.0.2-beta`。目前主要面向 Windows 10/11。

## 功能

- 按年、月组织照片和视频的时间线。
- 根据 EXIF GPS 在离线地图上展示拍摄地点。
- 中国省、市、县三级行政边界和中文地名。
- 地图缩放以鼠标位置为中心，底部时间轴支持滚轮切换月份。
- 地图聚合点可切换“照片数量”或“缩略图”显示。
- 递归扫描相册，不要求固定的文件夹结构。
- 支持多个相册根目录，可在设置中切换；移动硬盘或 NAS 离线时保留已有索引。
- 根据 EXIF、媒体元数据、文件名和目录推断拍摄时间。
- 增量索引，只重新处理发生变化的文件。
- 采用 320px 轻量缩略图，当前屏幕优先生成并在后台预热邻近内容。
- 优先复用内嵌预览和 Windows Shell 缓存，打开查看器时仍读取原始清晰度媒体。
- 照片与视频只读查看、前后导航和媒体详情。
- 只读索引 Obsidian Daily Notes，在独立日记界面联动当天照片与附件。
- 从照片查看器发现当日日记，并可跳回 Obsidian 打开原始笔记。
- 统一设置页面管理相册、日记、增量索引、地图标记和缩略图缓存。
- 发布版不依赖开发机路径，可在其他 Windows 电脑选择任意可读相册目录。

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

## 打包与部署

### 1. 准备打包环境

首次打包前确认已经安装：

- Node.js LTS，并可运行 `corepack`。
- Rust stable，推荐通过 rustup 安装 MSVC 工具链。
- Visual Studio Build Tools，勾选“使用 C++ 的桌面开发”和 Windows SDK。
- Microsoft Edge WebView2 Runtime。

在 PowerShell 中检查主要工具：

```powershell
node --version
corepack --version
rustc --version
cargo --version
```

项目使用 Corepack 调用 pnpm，不需要单独全局安装 `pnpm`。

### 2. 安装依赖并验证

```powershell
corepack pnpm install
corepack pnpm build
corepack pnpm test:rust
```

如果项目位于中文路径或同步盘中，Rust 的大量中间文件可能被同步软件占用。`package.cmd` 已将 Cargo 构建目录放到 `E:\TimeAlbumBuild\time-album`，以降低路径长度和文件锁问题。

### 3. 生成 Windows 安装包

```powershell
.\package.cmd
```

安装包默认输出到：

```text
E:\TimeAlbumBuild\time-album\release\bundle\nsis\
```

其中 `TimeTravel_<版本>_x64-setup.exe` 是可以发送给其他 Windows 用户的 NSIS 安装程序。源码和原始媒体不会被打入安装包。

### 4. 分发与安装

1. 将生成的 `TimeTravel_<版本>_x64-setup.exe` 发给用户。
2. 用户运行安装程序并按向导完成安装。
3. 首次启动后进入“设置”，添加一个或多个相册目录。
4. 对每个新目录执行一次扫描；以后使用增量扫描即可。
5. Obsidian 日记为可选功能，可在设置中选择 Daily Notes 目录。

应用的数据库、配置和缩略图缓存保存在 Windows 应用数据目录，不会写入相册或 Obsidian Vault。移动硬盘或网络目录暂时不可用时，相册会显示为离线；恢复连接后可直接切换回来，原索引不会被删除。

### 5. 发布前校验

建议计算安装包哈希，便于接收者验证文件完整性：

```powershell
Get-FileHash -Algorithm SHA256 "E:\TimeAlbumBuild\time-album\release\bundle\nsis\TimeTravel_0.0.2-beta_x64-setup.exe"
```

当前安装包尚未加入商业代码签名，因此 Windows SmartScreen 可能显示未知发布者。正式公开发布前建议购买代码签名证书，并进一步接入自动更新。

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
- 缩略图缓存容量上限与自动清理策略。
- 数据库备份、恢复和版本迁移。
- 年份快速跳转、地点搜索和快捷键。
- 代码签名、自动更新和便携版。

## 作者

`freestar`
