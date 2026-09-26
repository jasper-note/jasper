// saveShortcut.ts：Cmd/Ctrl+S「保存当前笔记」快捷键判定的纯逻辑单测。
import { describe, it, expect } from 'vitest'
import { isSaveShortcut, type ModKeyEvent } from './saveShortcut'

const ev = (over: Partial<ModKeyEvent> = {}): ModKeyEvent => ({
	key: 's',
	metaKey: false,
	ctrlKey: false,
	altKey: false,
	shiftKey: false,
	...over,
})

describe('isSaveShortcut', () => {
	it('macOS：Cmd+S 命中', () => {
		expect(isSaveShortcut(ev({ metaKey: true }))).toBe(true)
	})

	it('Windows/Linux：Ctrl+S 命中', () => {
		expect(isSaveShortcut(ev({ ctrlKey: true }))).toBe(true)
	})

	it('Caps Lock 导致的大写 S 同样命中', () => {
		expect(isSaveShortcut(ev({ metaKey: true, key: 'S' }))).toBe(true)
	})

	it('无修饰键的 s 不命中（正常打字）', () => {
		expect(isSaveShortcut(ev())).toBe(false)
	})

	it('其它键不命中', () => {
		expect(isSaveShortcut(ev({ metaKey: true, key: 'b' }))).toBe(false)
	})

	it('Shift/Alt 组合留给浏览器（另存为等）', () => {
		expect(isSaveShortcut(ev({ metaKey: true, shiftKey: true }))).toBe(false)
		expect(isSaveShortcut(ev({ ctrlKey: true, altKey: true }))).toBe(false)
	})
})