// events.ts（SSE 客户端）：解析 change 帧、坏帧忽略、幂等连接、重连合成 library reload。
// jsdom 无 EventSource → 用可手动触发的替身。
import { describe, it, expect, vi, beforeEach } from 'vitest'

class FakeEventSource {
	static instances: FakeEventSource[] = []
	url: string
	onopen: ((e?: unknown) => void) | null = null
	closed = false
	private listeners = new Map<string, ((e: MessageEvent) => void)[]>()
	constructor(url: string) {
		this.url = url
		FakeEventSource.instances.push(this)
	}
	addEventListener(type: string, fn: (e: MessageEvent) => void) {
		const arr = this.listeners.get(type) ?? []
		arr.push(fn)
		this.listeners.set(type, arr)
	}
	close() {
		this.closed = true
	}
	emit(type: string, data: string) {
		for (const fn of this.listeners.get(type) ?? []) fn({ data } as MessageEvent)
	}
}

async function freshModule() {
	vi.resetModules() // 模块级 source 单例，逐测试隔离
	return await import('./events')
}

beforeEach(() => {
	FakeEventSource.instances = []
	vi.stubGlobal('EventSource', FakeEventSource)
})

describe('connectEvents', () => {
	it('订阅 /api/events 并把 change 帧解析回调；坏帧忽略', async () => {
		const { connectEvents } = await freshModule()
		const got: unknown[] = []
		expect(connectEvents((ev) => got.push(ev))).toBe(true)

		const es = FakeEventSource.instances[0]
		expect(es.url).toBe('/api/events')
		es.emit('change', '{"kind":"note","op":"upsert","id":"abc"}')
		es.emit('change', 'not-json') // 不应抛、不应回调
		es.emit('change', '{"kind":"folder","op":"upsert","id":"f1"}')

		expect(got).toEqual([
			{ kind: 'note', op: 'upsert', id: 'abc' },
			{ kind: 'folder', op: 'upsert', id: 'f1' },
		])
	})

	it('幂等：重复连接返回 false 且不再建新连接', async () => {
		const { connectEvents } = await freshModule()
		expect(connectEvents(() => {})).toBe(true)
		expect(connectEvents(() => {})).toBe(false)
		expect(FakeEventSource.instances).toHaveLength(1)
	})

	it('重连成功（非首连）合成 library reload 兜底漏掉的事件', async () => {
		const { connectEvents } = await freshModule()
		const got: unknown[] = []
		connectEvents((ev) => got.push(ev))
		const es = FakeEventSource.instances[0]

		es.onopen?.() // 首连：不合成
		expect(got).toEqual([])
		es.onopen?.() // 断线重连：合成全量刷新
		expect(got).toEqual([{ kind: 'library', op: 'reload', id: '' }])
	})

	it('带会话 token 时走查询串（EventSource 不能带 Authorization 头）', async () => {
		localStorage.setItem('jasper.token', 'abc123')
		try {
			const { connectEvents } = await freshModule()
			connectEvents(() => {})
			expect(FakeEventSource.instances[0].url).toBe('/api/events?token=abc123')
		} finally {
			localStorage.removeItem('jasper.token')
		}
	})

	it('token 变化（登录/登出）→ 关旧连接换新的，拿回细粒度事件', async () => {
		const mod = await freshModule()
		mod.connectEvents(() => {})
		expect(FakeEventSource.instances[0].url).toBe('/api/events')

		const api = await import('./api')
		api.setAuthToken('tok')
		expect(mod.connectEvents(() => {})).toBe(true)
		expect(FakeEventSource.instances[0].closed).toBe(true)
		expect(FakeEventSource.instances[1].url).toBe('/api/events?token=tok')
		// 同一 token 再连 → 幂等
		expect(mod.connectEvents(() => {})).toBe(false)
		expect(FakeEventSource.instances).toHaveLength(2)
		api.setAuthToken(null)
	})

	it('disconnect 后可重新连接', async () => {
		const { connectEvents, disconnectEvents } = await freshModule()
		connectEvents(() => {})
		disconnectEvents()
		expect(FakeEventSource.instances[0].closed).toBe(true)
		expect(connectEvents(() => {})).toBe(true)
		expect(FakeEventSource.instances).toHaveLength(2)
	})
})
