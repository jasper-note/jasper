// 编辑器工具的抽象：句柄(EditorHandle) + 命令(EditorCommand)。
// 命令声明式（id/图标/文案/所属模式/run），不感知底层引擎；具体引擎实现 EditorHandle。
// 这层刻意对齐 plugin-design.md 的 contributes.command + contributes.toolbar{location}，
// 将来插件可用同一个 registerEditorCommand 贡献工具，宿主按 modes 过滤后渲染到 note-toolbar。

import type { BlockKind } from './markdown'

// 统一到 CodeMirror 6 后只有一个编辑器实例，'source'（源码）与 'live'（Live Preview
// 实时预览）是同一实例的两种视图；命令对两者通用（见 commands.ts 的 modes）。
export type EditorMode = 'source' | 'live'

// 由统一的 CodeMirror 编辑器实现（源码与 Live Preview 共用同一实例）；命令通过它操作当前编辑器。
export interface EditorHandle {
	readonly mode: EditorMode
	focus(): void
	// 聚焦并把光标移到正文开头（标题栏回车进正文用）。
	focusStart(): void
	getValue(): string
	setValue(md: string): void
	// 行内包裹：选区两侧加 marker（如 `**`）；无选区则插占位并选中；已包裹则去除（切换）。
	wrapInline(marker: string, placeholder?: string): void
	// 任意前后缀包裹（如链接 `[text](url)`），不做切换。
	wrapAround(before: string, after: string, placeholder?: string): void
	// 对选区所在行切换块级前缀（标题/引用/列表/待办）。
	applyBlock(kind: BlockKind): void
	// 光标处插入片段（替换选区）。
	insert(text: string): void
}

export interface EditorCommand {
	id: string
	icon: string
	title: string // i18n key
	group: 'block' | 'inline' | 'insert' | 'action'
	modes: EditorMode[] // 命令适用的编辑模式；工具栏据此过滤
	run(handle: EditorHandle): void
}
