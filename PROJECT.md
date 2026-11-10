# flyreader — 工作区

这是 flyreader 的专属工作区。以后聊这个项目，就在这个项目文件夹和同名聊天里进行。

- **仓库**：https://github.com/17472439chaochao-lab/flyreader
- **分支**：main（拉取时最新提交 `a8a8688`，加入软件运营界面截图）
- **是什么**：Rust 重写版跨平台开源在线小说阅读器，支持 Mac / Windows / Linux
- **技术栈**：Tauri 2 + React 18 + TypeScript + Rust，Vite 5，SQLite（rusqlite）
- **版本**：v1.2.0，MIT 许可证，作者飞鸟传说
- **功能**：导入 Legado 书源、多书源并发搜索、书架、离线缓存、本地 TXT / EPUB / Markdown 阅读、老板键、主题

## 本地开发

```bash
npm install
npm run dev          # tauri dev：Vite（127.0.0.1:1420）+ Rust 端
npm run dev:fe       # 只起前端
cargo test --lib     # 93 个单元测试（在 src-tauri 目录下）
npm run build        # 打包
```

环境要求：Rust 1.77+、Node.js 18+（推荐 20 LTS），macOS 需 Xcode Command Line Tools。

## 约定

- 以后这个项目的改动都只改这个文件夹，不动别的站点。
- 同步上游用 `git pull`；改完要推回 GitHub 时，先确认分支和改动内容。
- 注意：这里是云端 Linux 环境，不能直接打出 macOS 的 dmg；打包发布建议在你的 Intel Mac 上跑 `npm run build`。

## zip 依赖决策（2026-11-10，子代理 B）

- 保持 `zip 0.6.6` 不动，未升级到 9.x。原因：本机 VM 无 cargo，无法编译验证大版本升级；
  9.x 的 `ZipArchive::new` / `by_name` / `by_index_raw` 从公开文档看同名存在，
  但 0.6→9.x 跨多个大版本，`FileOptions` 等周边 API 与默认 feature 很可能有破坏性变化，
  不经编译验证就升级可能破坏 EPUB 解析（测试里也在用 `zip::write::ZipWriter`）。
- 0.6.6 已知的 advisory 涉及 `extract()` / AES 解密入口，本项目未调用这两个入口
  （只用 `ZipArchive::new` / `by_name` / `by_index_raw` 读条目），且已有条目数 /
  解压总量 / 压缩比三重 zip bomb 防护；未跑过 `cargo audit`，不要以此为准声称已审计。
- 待在有 Rust 工具链的机器上验证通过后再升级（见 `src-tauri/src/local_books/epub_parse.rs` 顶部注释）。
