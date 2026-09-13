# Tauri + SvelteKit + TypeScript

This template should help get you started developing with Tauri, SvelteKit and TypeScript in Vite.

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).

## 更新说明（v0.1.25）

- 书库刷新范围 = 已入库记录所在目录 ∪ 已登记书库根目录的一级子目录；
- 二级及更深目录不因刷新入库；
- 刷新不做任何实时文件监听；
- 刷新只做索引对账，不生成封面/字数等元数据，也不发起任何 AI 请求；
- 启动扫描触发的本地元数据队列与刷新无关。
