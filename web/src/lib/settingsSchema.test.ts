import { describe, it, expect } from 'vitest'
import {
	evalShowIf,
	filterSections,
	buildRequestBody,
	interpretResult,
	fillPlaceholders,
	isDisplayField,
	maskSecrets,
	actionsFor,
	chipGroups,
	type SettingsSection,
	type SettingsAction,
	type SettingsField,
} from './settingsSchema'

describe('evalShowIf', () => {
	it('undefined condition is always visible', () => {
		expect(evalShowIf(undefined, {})).toBe(true)
	})
	it('equals matches exact value', () => {
		expect(evalShowIf({ field: 's', equals: 'local' }, { s: 'local' })).toBe(true)
		expect(evalShowIf({ field: 's', equals: 'local' }, { s: 'webdav' })).toBe(false)
	})
	it('in / not_in match membership', () => {
		expect(evalShowIf({ field: 'm', in: ['a', 'b'] }, { m: 'a' })).toBe(true)
		expect(evalShowIf({ field: 'm', in: ['a', 'b'] }, { m: 'c' })).toBe(false)
		// not_in: 插件 provider key（非 local/webdav）→ 显示 provider-config
		expect(evalShowIf({ field: 's', not_in: ['local', 'webdav'] }, { s: 'plugin:x:y' })).toBe(true)
		expect(evalShowIf({ field: 's', not_in: ['local', 'webdav'] }, { s: 'local' })).toBe(false)
	})
	it('truthy treats empty string / false as hidden', () => {
		expect(evalShowIf({ field: 'p', truthy: true }, { p: true })).toBe(true)
		expect(evalShowIf({ field: 'p', truthy: true }, { p: 'anthropic' })).toBe(true)
		expect(evalShowIf({ field: 'p', truthy: true }, { p: '' })).toBe(false)
		expect(evalShowIf({ field: 'p', truthy: true }, { p: false })).toBe(false)
	})
	// truthy:false = 「没值时才显示」，用于 MCP 段的「生成 API key」按钮（已生成则换成「重新生成」）
	it('truthy:false shows only when the value is absent', () => {
		expect(evalShowIf({ field: 'k', truthy: false }, { k: false })).toBe(true)
		expect(evalShowIf({ field: 'k', truthy: false }, { k: '' })).toBe(true)
		expect(evalShowIf({ field: 'k', truthy: false }, {})).toBe(true)
		expect(evalShowIf({ field: 'k', truthy: false }, { k: true })).toBe(false)
		expect(evalShowIf({ field: 'k', truthy: false }, { k: 'jasper_mcp_x' })).toBe(false)
	})
})

// 用非 i18n 键的字面标题：resolveLabel 对未知键回退原串，故搜索行为与语言无关。
const sections: SettingsSection[] = [
	{
		id: 'data-source',
		title_key: 'Data source',
		icon: 'folder',
		fields: [{ key: 'local_path', type: 'text', label_key: 'Folder path' }],
		search_keys: ['WebDAV'],
	},
	{
		id: 'ai',
		title_key: 'AI',
		icon: 'sparkles',
		fields: [{ key: 'api_key', type: 'secret', label_key: 'API key' }],
	},
]

describe('filterSections', () => {
	it('empty query returns all', () => {
		expect(filterSections(sections, '').map((s) => s.id)).toEqual(['data-source', 'ai'])
		expect(filterSections(sections, '  ').map((s) => s.id)).toEqual(['data-source', 'ai'])
	})
	it('matches section title case-insensitively', () => {
		expect(filterSections(sections, 'data').map((s) => s.id)).toEqual(['data-source'])
	})
	it('matches a field label', () => {
		expect(filterSections(sections, 'api key').map((s) => s.id)).toEqual(['ai'])
	})
	it('matches search_keys', () => {
		expect(filterSections(sections, 'webdav').map((s) => s.id)).toEqual(['data-source'])
	})
	it('no match returns empty', () => {
		expect(filterSections(sections, 'zzz')).toEqual([])
	})
})

const connect: SettingsAction = {
	id: 'connect',
	label_key: 'Connect',
	request: { method: 'PUT', url: '/api/config', convention: 'config-result' },
	on_success: 'reload',
}
const save: SettingsAction = {
	id: 'save',
	label_key: 'Save',
	request: { method: 'PUT', url: '/api/auth/settings', convention: 'status' },
	on_success: 'relogin',
}
const clear: SettingsAction = {
	id: 'clear',
	label_key: 'Clear',
	request: { method: 'PUT', url: '/api/auth/settings', convention: 'status', extra: { clear_password: true } },
	submit: false,
}

describe('buildRequestBody', () => {
	it('data-source local → connect payload with create_new derived', () => {
		const body = buildRequestBody('data-source', connect, {
			create_new: 'existing',
			source_type: 'local',
			local_path: '/notes',
		})
		expect(body).toMatchObject({
			source_type: 'local',
			local_path: '/notes',
			plugin_id: '',
			plugin_storage: '',
			create_new: false,
			read_only: false,
		})
	})
	it('data-source plugin provider → splits key + nests plugin_config + create_new', () => {
		const body = buildRequestBody('data-source', connect, {
			create_new: 'new',
			source_type: 'plugin:webdav-storage:webdav',
			plugin_config: { url: 'https://x/' },
		})
		expect(body).toMatchObject({
			source_type: 'plugin',
			plugin_id: 'webdav-storage',
			plugin_storage: 'webdav',
			plugin_config: { url: 'https://x/' },
			create_new: true,
		})
	})
	it('non-data-source section posts field values as-is', () => {
		const values = { passwordless_read: true, list_mode: 'whitelist', folder_list: ['a'], password: 'x' }
		expect(buildRequestBody('access-control', save, values)).toEqual(values)
	})
	it('submit:false action sends only request.extra', () => {
		expect(buildRequestBody('access-control', clear, { password: 'x', list_mode: 'none' })).toEqual({
			clear_password: true,
		})
	})
})

// 展示型字段（MCP 段的端点地址 / claude mcp add 命令 / 工具清单）：服务端只下发模板，
// 运行时信息（访问地址、会话 token）由前端补；且这些字段不该回传给服务端。
describe('display fields (copy/note/chips)', () => {
	it('classifies copy/note/chips as display-only', () => {
		expect(isDisplayField('copy')).toBe(true)
		expect(isDisplayField('note')).toBe(true)
		expect(isDisplayField('chips')).toBe(true)
		expect(isDisplayField('text')).toBe(false)
		expect(isDisplayField('bool')).toBe(false)
	})

	it('fills {origin} and {header} with runtime values', () => {
		expect(fillPlaceholders('{origin}/mcp', 'http://127.0.0.1:27583', null)).toBe(
			'http://127.0.0.1:27583/mcp',
		)
		expect(
			fillPlaceholders('claude mcp add --transport http jasper {origin}/mcp{header}', 'https://n.example', 'tok123'),
		).toBe('claude mcp add --transport http jasper https://n.example/mcp --header "Authorization: Bearer tok123"')
	})

	it('drops the auth header entirely when there is no token', () => {
		// 未设访问密码时命令里不该出现空的 Authorization 头
		const out = fillPlaceholders('cmd {origin}/mcp{header}', 'http://h', null)
		expect(out).toBe('cmd http://h/mcp')
		expect(out).not.toContain('Authorization')
	})

	it('replaces every occurrence of a placeholder', () => {
		expect(fillPlaceholders('{origin} and {origin}', 'X', null)).toBe('X and X')
	})

	it('excludes display fields from the request body', () => {
		const fields: SettingsField[] = [
			{ key: 'enabled', type: 'bool' },
			{ key: 'endpoint', type: 'copy' },
			{ key: 'note', type: 'note' },
			{ key: 'tools', type: 'chips' },
		]
		const values = {
			enabled: true,
			endpoint: '{origin}/mcp',
			note: 'text',
			tools: [{ label_key: 'settings.mcp.toolsRead', items: ['search_notes'] }],
		}
		expect(buildRequestBody('mcp', save, values, fields)).toEqual({ enabled: true })
	})
})

// 密钥遮挡：API Key 与含 key 的命令默认显示圆点，复制仍复制原文（遮挡只作用于显示）。
describe('maskSecrets', () => {
	const key = 'jasper_mcp_' + 'ab12'.repeat(16)

	it('masks every occurrence and keeps the last 4 characters', () => {
		const cmd = `claude mcp add jasper http://h/mcp --header "Authorization: Bearer ${key}"`
		const out = maskSecrets(cmd, [key])
		expect(out).not.toContain(key)
		expect(out).toBe(`claude mcp add jasper http://h/mcp --header "Authorization: Bearer ${'•'.repeat(20)}ab12"`)
		expect(maskSecrets(`${key} ${key}`, [key])).toBe(`${'•'.repeat(20)}ab12 ${'•'.repeat(20)}ab12`)
	})

	it('masks short secrets entirely', () => {
		expect(maskSecrets('token=abc123', ['abc123'])).toBe('token=••••••')
	})

	it('skips empty secrets and leaves text without secrets unchanged', () => {
		// 没设 key 时 values.api_key 是空串、未登录时会话 token 是 null：都不能把文本搞乱
		expect(maskSecrets('http://h/mcp', ['', null, undefined])).toBe('http://h/mcp')
		expect(maskSecrets('http://h/mcp', [key])).toBe('http://h/mcp')
	})
})

describe('actionsFor', () => {
	const mk = (id: string, field?: string): SettingsAction => ({
		id,
		label_key: id,
		field,
		request: { method: 'PUT', url: '/x', convention: 'status' },
	})
	const actions = [mk('save'), mk('generate', 'api_key'), mk('clear_key', 'api_key'), mk('other', 'endpoint')]

	it('splits inline actions by field and keeps the rest for the footer', () => {
		expect(actionsFor(actions, 'api_key').map((a) => a.id)).toEqual(['generate', 'clear_key'])
		expect(actionsFor(actions, 'endpoint').map((a) => a.id)).toEqual(['other'])
		expect(actionsFor(actions).map((a) => a.id)).toEqual(['save'])
		expect(actionsFor(undefined)).toEqual([])
	})
})

describe('chipGroups', () => {
	it('accepts well-formed groups and drops malformed ones', () => {
		const groups = [
			{ label_key: 'settings.mcp.toolsRead', items: ['get_note'] },
			{ label_key: 'settings.mcp.toolsDestructive', tone: 'danger', items: ['delete_note'] },
			{ items: ['no label'] },
			null,
		]
		expect(chipGroups(groups).map((g) => g.label_key)).toEqual([
			'settings.mcp.toolsRead',
			'settings.mcp.toolsDestructive',
		])
	})

	it('falls back to nothing for an older server that sent a plain string', () => {
		expect(chipGroups('search_notes、get_note')).toEqual([])
		expect(chipGroups(undefined)).toEqual([])
	})
})

describe('interpretResult', () => {
	it('config-result reads body.ok', () => {
		expect(interpretResult('config-result', true, { ok: true, notes: 3 })).toEqual({ ok: true })
		expect(interpretResult('config-result', true, { ok: false, error: 'bad path' })).toEqual({
			ok: false,
			error: 'bad path',
		})
	})
	it('status uses HTTP ok, extracting message/error on failure', () => {
		expect(interpretResult('status', true, null)).toEqual({ ok: true })
		expect(interpretResult('status', false, { message: 'invalid' })).toEqual({ ok: false, error: 'invalid' })
		expect(interpretResult('status', false, { error: 'boom' })).toEqual({ ok: false, error: 'boom' })
	})
})
