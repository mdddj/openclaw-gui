# OpenClaw GUI

一个基于 Tauri v2 + React + TypeScript 的 OpenClaw 桌面控制台。

## 本地开发

```bash
pnpm install
pnpm tauri dev
```

## 构建

```bash
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml
pnpm tauri build
```

## 发布流程

仓库内已经配置好 GitHub Actions：

- `CI`：在 `main` 分支推送和 Pull Request 时执行 `pnpm build` 与 `cargo check`
- `Release`：推送 `v*` 标签时自动构建安装包并发布到 GitHub Releases

发布一个新版本的最短流程：

```bash
git tag v0.1.0
git push origin main --tags
```
