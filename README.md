# Tauri + SvelteKit + TypeScript

This template should help get you started developing with Tauri, SvelteKit and TypeScript in Vite.

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).

## 发布构建（务必遵守）

发布桌面版**必须**用 Tauri CLI，禁止裸 `cargo build --release`：

```bash
npm run release        # = tauri build（生成安装包 + exe）
npm run release:exe    # = tauri build --no-bundle（只出 exe，快速验证）
```

原因：前端产物（`frontendDist = ../build`）由 `beforeBuildCommand`（`npm run build`）在
`tauri build` 时构建并**嵌入**二进制；应用随后从内嵌资源加载页面。裸 `cargo build --release`
**不会**触发 `beforeBuildCommand`，产出的二进制会把 WebView 指向开发地址
`devUrl`（`http://localhost:1420`）。脱离 `npm run tauri dev` 运行时该地址无人监听，
WebView 直接报 `ERR_CONNECTION_REFUSED`「无法访问此网站」。

判断某个 exe 是否为坏构建：启动后 WebView 若显示 localhost 的浏览器错误页，即为裸 cargo 产物；
正常产物应直接渲染书库界面。发布前用 `npm run release:exe` 重新产出即可修复。

## 更新说明（v0.1.25）

- 书库刷新范围 = 已入库记录所在目录 ∪ 已登记书库根目录的一级子目录；
- 二级及更深目录不因刷新入库；
- 刷新不做任何实时文件监听；
- 刷新只做索引对账，不生成封面/字数等元数据，也不发起任何 AI 请求；
- 启动扫描触发的本地元数据队列与刷新无关。
