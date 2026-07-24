# 时空相册

本地优先、媒体绝对只读的个人时间线与地图相册。

## 不可变安全规则

- 媒体只通过只读句柄打开。
- 不修改内容、EXIF、文件时间、名称或目录位置。
- 应用写入目标一旦解析到相册根目录内，后端立即拒绝。
- 数据库、缓存和设置必须位于系统应用数据目录。
- `viewTools` 自动从媒体扫描范围排除。

## 开发

```powershell
pnpm install
pnpm desktop:dev
```

前端检查：

```powershell
pnpm build
```

后端检查：

```powershell
pnpm test:rust
```
