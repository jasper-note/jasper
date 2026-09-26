// Cmd/Ctrl+S 在浏览器里默认是「保存网页」（弹出另存为对话框）——对 SPA 毫无意义。
// 应用应当自己接管：拦下默认行为，并立即冲刷当前笔记的未保存改动（接线在 App.svelte 的 window 监听）。
// 判定抽成纯逻辑便于单测（keydown 事件只用到这几个字段）。

export interface ModKeyEvent {
	key: string
	metaKey: boolean
	ctrlKey: boolean
	altKey: boolean
	shiftKey: boolean
}

// 是否「保存当前笔记」快捷键：Cmd+S（macOS）/ Ctrl+S（Windows/Linux）。
// 带 Alt/Shift 的组合不抢，留给浏览器（如 Cmd+Shift+S = 另存为）。
export function isSaveShortcut(e: ModKeyEvent): boolean {
	if (e.altKey || e.shiftKey) return false
	if (!e.metaKey && !e.ctrlKey) return false
	return e.key.toLowerCase() === 's'
}