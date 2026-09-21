// 服务器驱动的设置描述符（GET /api/settings/schema）的类型与纯逻辑。
// 服务器下发「分区目录 + 字段 schema + 当前值 + 动作」，前端用单一通用渲染器
// （SettingsSection.svelte）渲染，无需在前端硬编码有哪些设置/字段/顺序/可用性。
// 这里只放类型与可 Vitest 直接覆盖的纯函数（无 DOM/组件）；渲染在 SettingsSection.svelte。

import { t } from './i18n.svelte'
import type { MsgKey } from './messages'

// 条件显隐：show_if 未定义 → 恒显示。field 指向同分区另一字段（或 values 里的只读标记如 password_set）。
export type ShowIf = {
	field: string
	equals?: string
	in?: string[]
	not_in?: string[]
	truthy?: boolean
}

export type SettingsFieldType =
	| 'text'
	| 'secret'
	| 'multiline'
	| 'number'
	| 'bool'
	| 'enum'
	| 'notebook-multiselect'
	| 'theme'
	| 'language'
	| 'provider-config'
	// 只读展示 + 一键复制（如 MCP 端点地址、`claude mcp add` 命令）。值不参与提交。
	| 'copy'
	// 纯展示文本（如 MCP 已暴露的工具清单）。值不参与提交。
	| 'note'

export interface SettingsFieldOption {
	value: string
	label_key?: string // i18n 键；未知回退原串（如 provider 字面名 'Anthropic'）
}

export interface SettingsField {
	key: string
	type: SettingsFieldType
	label_key?: string
	desc_key?: string
	placeholder_key?: string
	empty_key?: string // notebook-multiselect 无候选时的空态文案键
	default?: unknown
	required?: boolean
	options?: SettingsFieldOption[]
	options_source?: 'storage_providers' | 'folders' // 动态选项：前端解析
	writeonly?: boolean // secret 不回显；已设时提示留空保持不变
	set_flag?: string // 指向 values 里的「已设置」布尔键（如 password_set）
	client_store?: string // client 作用域字段：值存 localStorage 的键
	show_if?: ShowIf
}

export interface SettingsRequest {
	method: string
	url: string
	convention: 'config-result' | 'status'
	extra?: Record<string, unknown>
}

export interface SettingsAction {
	id: string
	label_key: string
	variant?: 'primary' | 'danger' | 'default'
	request: SettingsRequest
	on_success?: 'reload' | 'relogin' | 'saved' | 'none'
	show_if?: ShowIf
	submit?: boolean // false = 只发 request.extra，不带字段值（如清除密码）
}

export interface SettingsSection {
	id: string
	title_key: string
	icon: string
	scope?: 'server' | 'client'
	desc_key?: string
	fields: SettingsField[]
	values?: Record<string, unknown> // server 作用域当前值（含 secret）
	actions?: SettingsAction[]
	search_keys?: string[]
}

export interface SettingsSchema {
	sections: SettingsSection[]
}

/** i18n 键解析：已知键取译文，未知（如 provider 字面名 'Anthropic'）回退原串。 */
export function resolveLabel(key: string | undefined): string {
	if (!key) return ''
	return t(key as MsgKey)
}

/** 展示型字段（copy/note）：只读，不参与动作提交的载荷。 */
export function isDisplayField(type: SettingsFieldType): boolean {
	return type === 'copy' || type === 'note'
}

/**
 * 填充服务端下发的运行时占位符。服务端不知道客户端是从哪个地址访问它的，
 * 也不该知道浏览器里的会话 token，所以它只下发模板，由这里补齐：
 * - `{origin}` → 当前页面来源（如 `http://127.0.0.1:27583`）
 * - `{header}` → 有会话 token 时的 `--header "Authorization: Bearer <token>"`，否则空串
 *   （未设访问密码时本就不需要这个头，命令里也不该出现）
 */
export function fillPlaceholders(tpl: string, origin: string, token: string | null): string {
	const header = token ? ` --header "Authorization: Bearer ${token}"` : ''
	return tpl.replaceAll('{origin}', origin).replaceAll('{header}', header)
}

/** 条件显隐求值：show_if 未定义 → 恒显示。字段值取自当前表单值对象。 */
export function evalShowIf(cond: ShowIf | undefined, values: Record<string, unknown>): boolean {
	if (!cond) return true
	const v = values[cond.field]
	if (cond.equals !== undefined) return v === cond.equals
	if (cond.in) return typeof v === 'string' && cond.in.includes(v)
	if (cond.not_in) return !(typeof v === 'string' && cond.not_in.includes(v))
	if (cond.truthy) return Boolean(v)
	return true
}

/** 分区可搜索文本：标题 + 描述 + 各字段标签/描述/选项 + search_keys，全部解析成当前语言并小写。 */
export function sectionSearchText(s: SettingsSection): string {
	const parts: string[] = [resolveLabel(s.title_key), resolveLabel(s.desc_key)]
	for (const f of s.fields) {
		parts.push(resolveLabel(f.label_key), resolveLabel(f.desc_key))
		for (const o of f.options ?? []) parts.push(resolveLabel(o.label_key))
	}
	for (const k of s.search_keys ?? []) parts.push(resolveLabel(k))
	return parts.filter(Boolean).join(' ').toLowerCase()
}

/** 搜索过滤：按查询串（不区分大小写）匹配分区可搜索文本；空查询 → 原样返回（保持顺序）。 */
export function filterSections(sections: SettingsSection[], query: string): SettingsSection[] {
	const q = query.trim().toLowerCase()
	if (!q) return sections
	return sections.filter((s) => sectionSearchText(s).includes(q))
}

/** 组装动作请求体。data-source 分区做载荷适配（拆插件 key、推导 create_new）；其余分区直接用字段值。
 *  `fields` 用于剔除展示型字段（copy/note）——它们是给人看的，不该回传给服务端。 */
export function buildRequestBody(
	sectionId: string,
	action: SettingsAction,
	values: Record<string, unknown>,
	fields: SettingsField[] = [],
): Record<string, unknown> {
	const extra = action.request.extra ?? {}
	if (action.submit === false) return { ...extra }
	if (sectionId === 'data-source') return { ...dataSourcePayload(values), ...extra }
	const payload = { ...values }
	for (const f of fields) if (isDisplayField(f.type)) delete payload[f.key]
	return { ...payload, ...extra }
}

// 数据源字段值 → PUT /api/config 载荷。source_type 为 'local'|'webdav'|'plugin:<id>:<contrib>'。
function dataSourcePayload(values: Record<string, unknown>): Record<string, unknown> {
	const st = String(values.source_type ?? 'local')
	const isPlugin = st !== 'local' && st !== 'webdav'
	const parts = isPlugin ? st.split(':') : []
	const pluginConfig =
		isPlugin && values.plugin_config && typeof values.plugin_config === 'object'
			? (values.plugin_config as Record<string, unknown>)
			: {}
	return {
		source_type: isPlugin ? 'plugin' : st,
		local_path: String(values.local_path ?? ''),
		webdav_url: String(values.webdav_url ?? ''),
		webdav_user: String(values.webdav_user ?? ''),
		webdav_pass: String(values.webdav_pass ?? ''),
		plugin_id: parts[1] ?? '',
		plugin_storage: parts[2] ?? '',
		plugin_config: pluginConfig,
		read_only: Boolean(values.read_only),
		create_new: values.create_new === 'new',
	}
}

/** 据响应约定判定成功/失败：config-result 读 body.ok；status 读 HTTP 状态。返回统一 {ok,error}。 */
export function interpretResult(
	convention: 'config-result' | 'status',
	httpOk: boolean,
	body: unknown,
): { ok: boolean; error?: string } {
	const b = (body ?? {}) as { ok?: boolean; error?: string; message?: string }
	if (convention === 'config-result') {
		if (httpOk && b.ok) return { ok: true }
		return { ok: false, error: b.error }
	}
	// status：非 2xx = 失败，取 message/error
	if (httpOk) return { ok: true }
	return { ok: false, error: b.message ?? b.error }
}

/** 客户端作用域字段：从 localStorage 读初值（无则 default）。 */
export function readClientValue(field: SettingsField): unknown {
	if (!field.client_store) return field.default
	try {
		const v = localStorage.getItem(field.client_store)
		if (v !== null) return v
	} catch {
		/* localStorage 不可用 → 用默认 */
	}
	return field.default
}

/** 客户端作用域字段：即时写 localStorage（无 save 动作）。 */
export function writeClientValue(field: SettingsField, value: unknown): void {
	if (!field.client_store) return
	try {
		localStorage.setItem(field.client_store, String(value))
	} catch {
		/* 忽略 */
	}
}
