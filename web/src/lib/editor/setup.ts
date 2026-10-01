// 编辑器基础扩展集：一个 CodeMirror 6 实例，源码/Live Preview 是同一实例的两种视图。
// 关键能力：tab 缩进、选中包围（closeBrackets + 自定义 * _ ~ = ` 与中文全角符号包裹）、markdown(GFM) 语法、
// 主题、列表续行等。模式/只读/插件扩展各用一个 compartment，运行时可热切换、不重建实例。

import {
	EditorView,
	keymap,
	drawSelection,
	dropCursor,
	highlightActiveLine,
	placeholder as cmPlaceholder,
	type KeyBinding,
} from '@codemirror/view'
import { EditorState, EditorSelection, Compartment, type Extension } from '@codemirror/state'
import { history, historyKeymap, defaultKeymap, indentWithTab } from '@codemirror/commands'
import { indentUnit, indentOnInput, bracketMatching } from '@codemirror/language'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
import { languages } from '@codemirror/language-data'
import { editorThemeExtensions } from './theme'
import { livePreview } from './livePreview'
import { editorExtensions } from './editorExtensions'
import type { EditorMode } from './types'

// 运行时可重构的三个维度
export const modeCompartment = new Compartment()
export const readOnlyCompartment = new Compartment()
export const pluginCompartment = new Compartment()

export const modeExtension = (mode: EditorMode): Extension => (mode === 'live' ? livePreview : [])
export const readOnlyExtension = (ro: boolean): Extension => [EditorState.readOnly.of(ro), EditorView.editable.of(!ro)]

// 选中后直接键入下列符号即包裹选区（键入的字符 → [左, 右]）；无选区照常输入，不自动补全配对。
// ( [ { ' " 由 closeBrackets 负责；markdown 的 closeBrackets 默认列表不含反引号，故 ` 放这里。
// 中文输入法下 " ' 键在左右引号间交替出字，敲到左引号或右引号都按同一对包裹。
// 用 Map 而非对象字面量：输入法一次提交的整词（如 "constructor"）不会命中原型链上的属性。
const WRAP = new Map<string, [string, string]>([
	['*', ['*', '*']],
	['_', ['_', '_']],
	['~', ['~', '~']],
	['=', ['=', '=']],
	['`', ['`', '`']],
	['（', ['（', '）']],
	['【', ['【', '】']],
	['「', ['「', '」']],
	['《', ['《', '》']],
	['“', ['“', '”']],
	['”', ['“', '”']],
	['‘', ['‘', '’']],
	['’', ['‘', '’']],
])
const wrapOnType = EditorView.inputHandler.of((view, _from, _to, text) => {
	const pair = WRAP.get(text)
	if (!pair) return false
	const [open, close] = pair
	const { state } = view
	if (state.selection.ranges.every((r) => r.empty)) return false // 无选区 → 正常输入
	view.dispatch(
		state.changeByRange((range) => {
			if (range.empty) return { range }
			return {
				changes: [
					{ from: range.from, insert: open },
					{ from: range.to, insert: close },
				],
				range: EditorSelection.range(range.from + open.length, range.to + open.length),
			}
		}),
		{ userEvent: 'input.wrap' },
	)
	return true
})

// Mod-b / Mod-i 快捷键：包裹选区（无选区则插入标记对，光标居中）
function wrapCmd(marker: string): (view: EditorView) => boolean {
	return (view) => {
		const { state } = view
		view.dispatch(
			state.changeByRange((range) => ({
				changes: [
					{ from: range.from, insert: marker },
					{ from: range.to, insert: marker },
				],
				range: EditorSelection.range(range.from + marker.length, range.to + marker.length),
			})),
			{ userEvent: 'input' },
		)
		return true
	}
}
const formatKeymap: KeyBinding[] = [
	{ key: 'Mod-b', run: wrapCmd('**') },
	{ key: 'Mod-i', run: wrapCmd('*') },
]

/** 基础扩展。updateListener / DOM 事件 / 文件上传由 build.ts 与组件负责。 */
export function baseExtensions(opts: { mode: EditorMode; readOnly: boolean; placeholderText?: string }): Extension {
	return [
		history(),
		drawSelection(),
		dropCursor(),
		EditorState.allowMultipleSelections.of(true),
		indentUnit.of('\t'),
		indentOnInput(),
		bracketMatching(),
		closeBrackets(),
		highlightActiveLine(),
		EditorView.lineWrapping,
		// codeLanguages：按 info string（```js/rust/…）懒加载对应语言，给围栏代码块做嵌套语法着色
		markdown({ base: markdownLanguage, codeLanguages: languages, addKeymap: true }),
		editorThemeExtensions,
		wrapOnType,
		keymap.of([indentWithTab, ...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...formatKeymap]),
		opts.placeholderText ? cmPlaceholder(opts.placeholderText) : [],
		modeCompartment.of(modeExtension(opts.mode)),
		readOnlyCompartment.of(readOnlyExtension(opts.readOnly)),
		pluginCompartment.of(editorExtensions()),
	]
}
