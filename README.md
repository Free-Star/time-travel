# 时空相册

本地优先、媒体绝对只读的个人时间线与地图相册。

当前已实现：

- Tauri 桌面应用与相册目录选择。
- SQLite 媒体索引、EXIF 时间/GPS 读取。
- 可取消的后台扫描与扫描进度。
- 基于路径、大小和修改时间的增量扫描。
- 照片缩略图与视频封面后台队列。
- 缩略图缓存状态、预览和一键清理。

## 不可变安全规则

- 媒体只通过只读句柄打开。
- 不修改内容、EXIF、文件时间、名称或目录位置。
- 应用写入目标一旦解析到相册根目录内，后端立即拒绝。
- 数据库、缓存和设置必须位于系统应用数据目录。
- `viewTools` 自动从媒体扫描范围排除。
- 视频只作为 FFmpeg 输入，生成结果写入应用缓存。

为规避 MSVC 在中文深路径下写入大型 PDB 的问题，Rust 的可再生构建缓存位于
`E:\TimeAlbumBuild\time-album`；源码仍全部位于本目录。

## 开发

```powershell
.\start.cmd
```

`start.cmd` 会通过 Node 自带的 Corepack 调用项目指定的 pnpm，不要求全局安装 pnpm。

前端检查：

```powershell
pnpm build
```

后端检查：

```powershell
pnpm test:rust
```
