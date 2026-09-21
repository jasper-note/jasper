// SSE 变更流客户端（GET /api/events）。服务端在一切写路径上广播 (kind, op, id)，
// 内容由调用方按需再拉——见 App.svelte 的去抖合并刷新。
// 断线由 EventSource 自动重连；重连间隙可能漏事件，故重连成功时合成一条
// library reload 交给调用方全量刷新兜底（服务端重启/网络抖动都被覆盖）。
import { getAuthToken, IS_WASM } from './api'

export type ChangeEvent = {
	kind: 'note' | 'folder' | 'tag' | 'library'
	op: 'upsert' | 'delete' | 'reload'
	id: string // kind=tag 时为受影响的笔记 id
}

let source: EventSource | null = null
let sourceToken: string | null = null // 当前连接所用的会话 token（变了要重连）

/** 订阅地址：EventSource 不能带自定义请求头（浏览器 API 限制），故会话 token 只能走查询串。
 *  不带 token 的连接在服务端算匿名；设了访问密码时匿名可见范围受限，服务端会把每条事件
 *  粗化成 library reload（见 server/src/api.rs events_sse），前端就会对每次自己的保存做一次
 *  全量重载——打开的笔记连同编辑器被销毁重建（表现为编辑区闪一下）。 */
function eventsUrl(token: string | null): string {
	return token ? `/api/events?token=${encodeURIComponent(token)}` : '/api/events'
}

/** 建立事件订阅（幂等：已用同一 token 连上/无后端 WASM 构建/环境不支持 → false）。
 *  登录/登出后 token 变化 → 自动换连接，拿到与新身份相符的事件粒度。 */
export function connectEvents(onChange: (ev: ChangeEvent) => void): boolean {
	if (IS_WASM || typeof EventSource === 'undefined') return false
	const token = getAuthToken()
	if (source) {
		if (sourceToken === token) return false
		disconnectEvents()
	}
	const es = new EventSource(eventsUrl(token))
	source = es
	sourceToken = token
	let openedOnce = false
	es.onopen = () => {
		// 重连成功（非首连）：间隙内的事件已丢 → 全量刷新
		if (openedOnce) onChange({ kind: 'library', op: 'reload', id: '' })
		openedOnce = true
	}
	es.addEventListener('change', (e) => {
		try {
			onChange(JSON.parse((e as MessageEvent).data) as ChangeEvent)
		} catch {
			/* 坏帧忽略 */
		}
	})
	// onerror 不关闭：EventSource 自带指数退避重连
	return true
}

/** 断开（测试/登出用）。 */
export function disconnectEvents() {
	source?.close()
	source = null
	sourceToken = null
}
