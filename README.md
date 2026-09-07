# BaiduPCS-Rust Desktop

基于 [Tauri v2](https://v2.tauri.app/) 深度整合与打包的 [BaiduPCS-Rust](https://github.com/komorebiCarry/BaiduPCS-Rust) 桌面客户端。

支持在 **Windows**（`.msi` / `.exe`）和 **Linux**（`.deb` / `.AppImage`）系统上提供轻量、高效、低资源占用的原生桌面体验。

---

## ✨ 桌面端专属特性

- 🚀 **极轻量原生架构**：底层直接内嵌 Rust Tokio 异步运行时与 Axum 服务，前端采用系统原生 WebView 渲染（Windows WebView2 / Linux WebKitGTK），无需臃肿的 Chromium 内核。
- 📌 **系统托盘集成 (System Tray)**：点击窗口关闭按钮（`X`）时自动最小化到系统右下角托盘，后台保持高速上传与下载不断连；右键托盘支持一键呼出、最小化或退出。
- 🛡️ **单实例守护 (Single Instance)**：自动防止多开冲突，重复启动时自动激活并前台聚焦已有窗口。
- ⚡ **开箱即用 & 优雅退出**：退出程序时自动安全关闭持久化管理器并保存所有下载/上传状态，杜绝数据损坏。
- 📦 **企业级原生安装包**：
  - **Windows**: 采用 WiX Toolset 生成标准 `.msi` 安装包（支持静默安装，自动注册至 Windows 系统“应用和功能”及控制面板），以及 NSIS `.exe` 安装程序。
  - **Linux**: 生成标准 `.deb` 安装包（自动注册至桌面应用启动菜单）以及免安装独立运行的 `.AppImage`。

---

## 🛠️ 本地开发与构建

### 1. 环境准备

- [Rust](https://rustup.rs/) (1.80+)
- [Node.js](https://nodejs.org/) (20+)
- Windows: 系统自带 WebView2（Windows 10/11 预装）
- Linux: 安装 WebKitGTK 开发库（如 Ubuntu 下 `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev`）

### 2. 克隆仓库与拉取上游子模块

```bash
git clone --recurse-submodules https://github.com/SunnyYYLin/BaiduPCS-Rust-Desktop.git
cd BaiduPCS-Rust-Desktop
```

### 3. 安装依赖

```bash
npm install
npm run install:frontend
```

### 4. 启动本地桌面端调试

```bash
npm run dev
```

### 5. 本地打包安装包

```bash
# Windows 打包生成 .msi 和 .exe 安装包
npm run tauri build -- --bundles msi,nsis

# Linux 打包生成 .deb 和 .AppImage
npm run tauri build -- --bundles deb,appimage
```

产物将生成在 `src-tauri/target/release/bundle/` 目录下。

---

## 🔄 同步上游更新

本项目通过 `git submodule` 持续跟踪上游 [komorebiCarry/BaiduPCS-Rust](https://github.com/komorebiCarry/BaiduPCS-Rust) 的最新迭代。若需拉取上游新特性：

```bash
git submodule update --remote --merge
npm run build:frontend
```

---

## 🤖 自动化发布 (CI/CD)

项目已配置 GitHub Actions 跨平台流水线 (`.github/workflows/release.yml`)。

只需推送版本标签（例如 `git tag v2.2.3 && git push origin v2.2.3`），GitHub Actions 将自动在云端多平台 Runner（Windows 和 Ubuntu）上并发编译，并将 `.msi`、`.exe`、`.deb`、`.AppImage` 自动发布至 GitHub Releases。

---

## 📄 开源许可证

本项目基于 Apache License 2.0 许可证开源。上游项目源码版权归原作者 [komorebiCarry](https://github.com/komorebiCarry) 所有。
