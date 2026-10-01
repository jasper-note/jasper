// EditorHandle 实现工厂：把工具栏命令 / 插件命令对「当前源码」的操作，
// 落到一个具体的 CodeMirror EditorView 上。源码模式与 Live Preview 共用同一个
// EditorView，故同一套实现两模式通用（不再有富文本/源码两套语义）。

import type { EditorView } from '@codemirror/view'
import { toggleBlockLines, type BlockKind } from './markdown'
import type { EditorHandle, EditorMode } from './types'

export function makeHandle(view: EditorView, getMode: () => EditorMode): EditorHandle {
	// 光标处插入文本（替换选区），插入后聚焦并把光标移到末尾。
	function insert(text: string) {
		const sel = view.state.selection.main
		view.dispatch({
			changes: { from: sel.from, to: sel.to, insert: text },
			selection: { anchor: sel.from + text.length },
		})
		view.focus()
	}

	// 对选区所在整行切换块级前缀（标题/引用/列表/待办），逻辑在纯函数 toggleBlockLines。
	function applyBlock(kind: BlockKind) {
		const { state } = view
		const range = state.selection.main
		const startLine = state.doc.lineAt(range.from)
		const endLine = state.doc.lineAt(range.to)
		const src: string[] = []
		for (let n = startLine.number; n <= endLine.number; n++) src.push(state.doc.line(n).text)
		view.dispatch({
			changes: { from: startLine.from, to: endLine.to, insert: toggleBlockLines(src, kind).join('\n') },
		})
		view.focus()
	}

	// 任意前后缀包裹（无选区则插占位并选中占位，便于直接改写）。
	function wrapAround(before: string, after: string, placeholder = '') {
		const sel = view.state.selection.main
		const inner = view.state.sliceDoc(sel.from, sel.to) || placeholder
		view.dispatch({
			changes: { from: sel.from, to: sel.to, insert: before + inner + after },
			selection: { anchor: sel.from + before.length, head: sel.from + before.length + inner.length },
		})
		view.focus()
	}

	// 行内标记（对称）：已被同标记包裹则去除（切换），否则包裹。
	function wrapInline(marker: string, placeholder = '') {
		const state = view.state
		const { from, to } = state.selection.main
		if (to > from) {
			const b = state.sliceDoc(Math.max(0, from - marker.length), from)
			const a = state.sliceDoc(to, Math.min(state.doc.length, to + marker.length))
			if (b === marker && a === marker) {
				view.dispatch({
					changes: [
						{ from: from - marker.length, to: from, insert: '' },
						{ from: to, to: to + marker.length, insert: '' },
					],
					selection: { anchor: from - marker.length, head: to - marker.length },
				})
				view.focus()
				return
			}
		}
		wrapAround(marker, marker, placeholder)
	}

	return {
		get mode() {
			return getMode()
		},
		focus: () => view.focus(),
		focusStart: () => {
			view.dispatch({ selection: { anchor: 0 }, scrollIntoView: true })
			view.focus()
		},
		getValue: () => view.state.doc.toString(),
		setValue: (md) => {
			view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: md } })
			view.focus()
		},
		wrapInline,
		wrapAround,
		applyBlock,
		insert,
	}
}
