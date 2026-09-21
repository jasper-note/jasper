// 后端 API 客户端。开发期经 Vite 代理到 27583，生产期同源访问。
import * as fsStore from './fsStore'
import { t } from './i18n.svelte'
import * as localStore from './localStore'
import type { Schema } from './schema'
import type { SettingsSchema } from './settingsSchema'

// 拖拽（移动）用的 dataTransfer MIME：笔记 / 笔记本各一种，放置目标据此区分。
export const NOTE_DND_TYPE = 'application/x-jasper-note-id'
export const FOLDER_DND_TYPE = 'application/x-jasper-folder-id'

export interface FolderNode {
  id: string
  title: string
  note_count: number
  children: FolderNode[]
}

export interface NoteSummary {
  id: string
  title: string
  updated_time: number
  parent_id: string
  is_todo: boolean
  todo_completed: boolean
  task_done: number // 正文任务清单已完成数
  task_total: number // 正文任务清单总数（0 = 无任务清单）
}

export interface FolderRef {
  id: string
  title: string
  parent_id: string
}

export interface TagInfo {
  id: string
  title: string
  note_count: number // 打了该标签且仍存在的笔记数
}

// 笔记上的标签（GET/POST /api/notes/{id}/tags 的元素）。
export interface TagRef {
  id: string
  title: string
}

export interface NoteDetail {
  id: string
  title: string
  body: string
  markup_language: number // 1=Markdown, 2=HTML
  parent_id: string
  created_time: number
  updated_time: number
  source_url: string
  is_todo: boolean
  todo_completed: boolean
}

export interface SourceConfig {
  source_type: string // 'local' | 'webdav' | 'plugin'
  local_path: string
  webdav_url: string
  webdav_user: string
  webdav_pass: string
  // source_type === 'plugin'（插件存储 provider，spec 0.2）
  plugin_id: string
  plugin_storage: string
  plugin_config: string // 服务端存 JSON 文本（GET /api/config 原样回显，含 secret，与 webdav_pass 同姿势）
  read_only: boolean // 只读模式：拒绝一切写操作
}

export interface StatusResp {
  configured: boolean
  source_type: string
  notes: number
  folders: number
  read_only: boolean // 服务端只读模式是否开启
  auth_enabled: boolean // 是否设了访问密码（受保护）
  authenticated: boolean // 本次会话是否已登录（携带有效 token）
  passwordless_read: boolean // 允许无密码阅读总开关
  version: string // 服务端版本（市场 UI 做 minHostVersion 兼容过滤）
}

// 访问控制设置（GET /api/auth/settings；密码只回 password_set 布尔，绝不回哈希）。
export type AuthListMode = 'none' | 'whitelist' | 'blacklist'
export interface AuthSettings {
  password_set: boolean
  passwordless_read: boolean
  list_mode: AuthListMode
  folder_list: string[] // 名单笔记本 id
}

// PUT /api/auth/settings 请求体。password 非空=设/改；clear_password=清除（关鉴权）。
export interface AuthSettingsUpdate {
  password?: string
  clear_password?: boolean
  passwordless_read: boolean
  list_mode: AuthListMode
  folder_list: string[]
}

export interface ConfigResult {
  ok: boolean
  error: string | null
  notes: number
  folders: number
}

// PUT /api/config 的请求体：plugin_config 以对象提交（服务端校验后规范化存储）。
export interface ApplyConfigReq extends Omit<SourceConfig, 'plugin_config'> {
  plugin_config: Record<string, unknown>
  create_new: boolean
}

// ---------- 插件（服务端 --features plugins；未编译时探测为不可用）----------

export interface ThemeContribution {
  id: string
  name: string
  base: 'light' | 'dark'
  css: string // 包内相对路径，经 pluginAssetUrl 取
}

// 语言包贡献（spec §3.10，0.4）：给应用增加一门界面语言；messages 指向 catalog JSON（相对路径，经资产端点取）。
export interface LocaleContribution {
  code: string // 语言代码（fr / ja / pt-BR）
  name: string // 该语言自称显示名（Français）
  base: 'en' | 'zh' // 缺失键回落语言
  messages: string // catalog JSON 包内相对路径
}

export interface StorageContribution {
  id: string
  name: string
  icon: string // 图标令牌名（--icon-*），空 = 用默认 plug
  config_schema: Schema
}

export interface CommandContribution {
  id: string
  title: string
  icon: string // 图标令牌名，空 = 默认
  target: 'backend' | 'builtin'
}

export interface ToolbarContribution {
  command: string
  location: 'note-toolbar' | 'topbar'
}

// 侧边栏面板贡献（spec §3.5，0.3）：静态 widget（交互经 command）或动态树（view → ui 端点）。
export interface SidebarContribution {
  id: string
  title: string
  icon: string // 图标令牌名，空 = 默认 plug
  widget: string // chat | list | tree | form | markdown | button
  command?: string
  view?: string
}

// 编辑器钩子贡献（spec §3.7，0.4 阶段 4）：编辑期把编辑缓冲文本交给 wasm 的 editor.transform 改写。
export interface EditorContribution {
  on: 'before-save' | 'input'
}

export interface PluginContributes {
  theme: ThemeContribution[]
  locale: LocaleContribution[]
  storage: StorageContribution[]
  command: CommandContribution[]
  toolbar: ToolbarContribution[]
  sidebar: SidebarContribution[]
  editor: EditorContribution[]
}

export interface PluginInfo {
  id: string
  name: string
  version: string
  api_version: string
  description: string
  author: string
  enabled: boolean
  has_backend: boolean
  capabilities: string[]
  hooks: string[]
  error: string | null
  contributes: PluginContributes
  settings_schema: Schema
  write_auto_approve: boolean // notes:write 的「写入免确认」（宿主托管，spec 0.3 §7）
}

// ---------- server-driven UI / 写提案 / AI（spec 0.3）----------

// UiNode 声明树（spec §9.3）：{type, props, children}，未知 type 前端安全忽略（连 children）。
export interface UiNode {
  type: string
  props: Record<string, unknown>
  children?: UiNode[]
}

// 写提案（spec §9.5 pending_writes 元素）：插件的 notes.upsert/create 默认不落盘，
// 由前端弹 diff 确认，同意后走普通 PUT/POST /api/notes*。
export interface PendingWrite {
  action: 'update' | 'create'
  plugin_id: string
  note: { id: string; parent_id: string; title: string; body: string }
  original: { title: string; body: string } | null // create 为 null
}

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant'
  content: string
}

// 宿主级 AI 配置（GET/PUT /api/ai/config；密钥存本机、插件不可见）。
export interface AiConfig {
  provider: '' | 'anthropic' | 'openai'
  base_url: string
  api_key: string
  model: string
}

// 命令/ui 端点的响应：0.3 起响应恒带 pending_writes（无提案 = 空数组）。
export interface PluginCommandResp {
  result: Record<string, unknown>
  pending_writes: PendingWrite[]
}

export interface PluginUiResp {
  ui: UiNode
  pending_writes: PendingWrite[]
}

export interface PluginsResp {
  host: { version: string; api_versions: string[] }
  plugins: PluginInfo[]
}

export interface PluginInstallResult {
  plugin: PluginInfo
  needs_consent: boolean
}

export interface PluginSettingsResp {
  values: Record<string, unknown>
  secret_set: Record<string, boolean> // secret 不回显，仅标记「已设置」
}

export interface ResourceUpload {
  id: string
  title: string
  mime: string
  file_extension: string
  size: number
  markdown: string // 可直接插入正文的引用片段
}

export interface ResourceInfo {
  id: string
  title: string
  mime: string
  file_extension: string
  size: number
  updated_time: number
  used_by: number // 引用该资源的笔记数（0 = 孤儿）
}

// 无后端（backendless）构建：读写全走浏览器内的 WASM（jasper-core 编译产物），不需任何 server。
// 两种：
// - VITE_DEMO=1 → 只读展示站（内置演示库，GitHub Pages 用）。
// - VITE_LOCAL=1 → 可写本地应用（从 IndexedDB 恢复/播种，读写持久）。
const DEMO = import.meta.env.VITE_DEMO === '1'
const LOCAL = import.meta.env.VITE_LOCAL === '1'
/// 无后端、走 WASM 的构建（展示站与本地应用都属此）。events/plugins 据此关闭。
export const IS_WASM = DEMO || LOCAL
/// 可写且持久的本地应用构建。
export const WASM_WRITABLE = LOCAL
/// 只读展示构建（供「演示预览」横幅 + 只读闸门用）。IS_DEMO ≡ IS_WASM && !WASM_WRITABLE。
export const IS_DEMO = IS_WASM && !WASM_WRITABLE

// WASM 实例（读 + 写）方法面——与 wasm/src/lib.rs 的 Demo 一一对应（避免依赖 gitignore 的生成类型）。
type WasmInstance = {
  folders(): string
  notes(f: string): string
  note(id: string): string
  search(q: string): string
  tags(): string
  notesByTag(tagId: string): string
  noteTags(noteId: string): string
  resources(): string
  snapshot(): string[]
  createNote(parentId: string, title: string, body: string, isTodo: boolean, now: number): string
  updateNote(id: string, title: string, body: string, now: number): string
  moveNote(id: string, newParent: string, now: number): string
  deleteNote(id: string): void
  createFolder(parentId: string, title: string, now: number): string
  renameFolder(id: string, title: string, now: number): string
  moveFolder(id: string, newParent: string, now: number): string
  deleteFolder(id: string): void
  addNoteTag(noteId: string, title: string, now: number): string
  removeNoteTag(noteId: string, tagId: string): string
  upsertResourceMeta(id: string, title: string, mime: string, ext: string, size: number, now: number): string
  renameResource(id: string, title: string, now: number): string
  deleteResource(id: string): void
}
type WasmModule = {
  default(): Promise<unknown>
  Demo: {
    new (): WasmInstance
    fromRaws(raws: string[]): WasmInstance
    seedItems(): string[]
  }
}

let _wasm: Promise<WasmModule> | null = null
function loadWasm(): Promise<WasmModule> {
  if (!_wasm) {
    _wasm = (async () => {
      const mod = (await import('../wasm-pkg/jasper_wasm.js')) as unknown as WasmModule
      await mod.default() // 加载并初始化 .wasm
      return mod
    })()
  }
  return _wasm
}

// 只读展示：内置演示库。
let _demo: Promise<WasmInstance> | null = null
function wasmDemo(): Promise<WasmInstance> {
  if (!_demo) _demo = loadWasm().then((m) => new m.Demo())
  return _demo
}

// ---------- 会话 token（访问鉴权）----------
// 登录成功后拿到的会话 token 存 localStorage，之后每个请求带 `Authorization: Bearer`。
// 读天然放行（服务端按可见范围过滤）；写/机密读需要它。不用 cookie（契合 permissive CORS + LAN http）。
const TOKEN_KEY = 'jasper.token'
let authToken: string | null = readStoredToken()

function readStoredToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY)
  } catch {
    return null
  }
}

export function getAuthToken(): string | null {
  return authToken
}

export function setAuthToken(token: string | null) {
  authToken = token
  try {
    if (token) localStorage.setItem(TOKEN_KEY, token)
    else localStorage.removeItem(TOKEN_KEY)
  } catch {
    /* localStorage 不可用（隐私模式等）→ 仅内存态 */
  }
}

// 组装请求头，附带当前 token（若有）。
export function authHeaders(base: Record<string, string> = {}): Record<string, string> {
  return authToken ? { ...base, Authorization: `Bearer ${authToken}` } : base
}

// token 失效（服务端重启/改密码）时的回调：由 App 注册以清态 + 提示重新登录。
let onAuthErrorCb: (() => void) | null = null
export function setAuthErrorHandler(cb: (() => void) | null) {
  onAuthErrorCb = cb
}
function authFailed() {
  setAuthToken(null)
  onAuthErrorCb?.()
}

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { headers: authHeaders() })
  if (!res.ok) {
    if (res.status === 401) authFailed()
    throw new Error(`${url} -> ${res.status}`)
  }
  return res.json() as Promise<T>
}

async function sendJson<T>(url: string, method: string, body: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: authHeaders({ 'Content-Type': 'application/json' }),
    body: JSON.stringify(body),
  })
  if (!res.ok) {
    if (res.status === 401) authFailed()
    throw new Error(`${method} ${url} -> ${res.status}`)
  }
  return res.json() as Promise<T>
}

// Joplin 内部资源/笔记链接 `:/<32hex>` 或 `joplin://<32hex>` → 解析出 id（无则 null）。
// 与 render.ts 的改写逻辑同源，供富文本编辑器把 :/id 映射成可显示 URL。
const RESOURCE_LINK = /^(?:joplin:\/\/|:\/)([0-9a-zA-Z]{32})(?:[#?].*)?$/
export function parseResourceId(url: string): string | null {
  const m = RESOURCE_LINK.exec((url || '').trim())
  return m ? m[1] : null
}

// 统计 markdown 任务清单（GFM checkbox）完成/总数 [done, total]。与 core::library::count_tasks 同义，
// 供编辑时按当前正文实时显示进度（列表项的计数由后端给）。
export function taskProgress(body: string): [number, number] {
  let done = 0
  let total = 0
  for (const line of (body || '').split('\n')) {
    const m = /^\s*[-*+] \[([ xX])\](?:\s|$)/.exec(line)
    if (!m) continue
    total++
    if (m[1] !== ' ') done++
  }
  return [done, total]
}

const httpApi = {
  status: () => getJson<StatusResp>('/api/status'),
  getConfig: () => getJson<SourceConfig>('/api/config'),
  saveConfig: (data: ApplyConfigReq) => sendJson<ConfigResult>('/api/config', 'PUT', data),

  // ---------- 服务器驱动的设置描述符 ----------
  // 探测坑同 plugins()：老服务器无此路由时 SPA fallback 回 200 的 index.html —— 必须查 content-type。
  // 机密读端点：token 失效 → 401 触发全局 authFailed（设置面板仅登录后可开）。
  settingsSchema: async (): Promise<SettingsSchema | null> => {
    try {
      const res = await fetch('/api/settings/schema', { headers: authHeaders() })
      if (res.status === 401) {
        authFailed()
        return null
      }
      const ct = res.headers.get('content-type') ?? ''
      if (!res.ok || !ct.includes('application/json')) return null
      return (await res.json()) as SettingsSchema
    } catch {
      return null
    }
  },
  // 通用设置动作：向描述符声明的端点发请求（带鉴权），返回 {ok,status,body} 供调用方按约定判定。
  sendSettingsAction: async (
    method: string,
    url: string,
    body: Record<string, unknown>,
  ): Promise<{ ok: boolean; status: number; body: unknown }> => {
    const res = await fetch(url, {
      method,
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(body),
    })
    if (res.status === 401) authFailed()
    const json = await res.json().catch(() => null)
    return { ok: res.ok, status: res.status, body: json }
  },

  // ---------- 访问鉴权（access control）----------
  // 登录：正确密码 → 存 token 返回 true；密码错 → false；其它错误抛异常。
  login: async (password: string): Promise<boolean> => {
    const res = await fetch('/api/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ password }),
    })
    if (res.status === 401) return false // 密码错误（不触发全局 authFailed）
    if (!res.ok) throw new Error(`login -> ${res.status}`)
    const body = (await res.json()) as { token: string }
    setAuthToken(body.token)
    return true
  },
  // 登出：吊销服务端会话 + 清本地 token。
  logout: async () => {
    try {
      await fetch('/api/auth/logout', { method: 'POST', headers: authHeaders() })
    } catch {
      /* 忽略网络错误——本地 token 照清 */
    }
    setAuthToken(null)
  },
  // 访问控制设置（需已登录/未设密码时可读）。密码只回 password_set。
  getAuthSettings: () => getJson<AuthSettings>('/api/auth/settings'),
  saveAuthSettings: (data: AuthSettingsUpdate) =>
    sendJson<AuthSettings>('/api/auth/settings', 'PUT', data),

  folders: () => getJson<FolderNode[]>('/api/folders'),
  createFolder: (data: { parent_id: string; title: string }) =>
    sendJson<FolderRef>('/api/folders', 'POST', data),
  renameFolder: (id: string, title: string) =>
    sendJson<FolderRef>(`/api/folders/${id}`, 'PUT', { title }),
  moveFolder: (id: string, parentId: string) =>
    sendJson<FolderRef>(`/api/folders/${id}/move`, 'PUT', { parent_id: parentId }),
  // 删除笔记本：服务端级联删掉子笔记本与其中全部笔记
  deleteFolder: async (id: string) => {
    const res = await fetch(`/api/folders/${id}`, { method: 'DELETE', headers: authHeaders() })
    if (!res.ok) throw new Error(`DELETE -> ${res.status}`)
  },
  notes: (folderId: string) =>
    getJson<NoteSummary[]>(`/api/notes?folder=${encodeURIComponent(folderId)}`),
  note: (id: string) => getJson<NoteDetail>(`/api/notes/${id}`),
  search: (q: string) => getJson<NoteSummary[]>(`/api/search?q=${encodeURIComponent(q)}`),
  // 标签视图（只读浏览）：列表 + 按标签过滤笔记
  tags: () => getJson<TagInfo[]>('/api/tags'),
  notesByTag: (tagId: string) =>
    getJson<NoteSummary[]>(`/api/tags/${encodeURIComponent(tagId)}/notes`),
  // 某笔记的标签 + 打/去标签（打标签按标题，标签不存在则服务端新建，兼容 Joplin）
  noteTags: (noteId: string) => getJson<TagRef[]>(`/api/notes/${noteId}/tags`),
  addNoteTag: (noteId: string, title: string) =>
    sendJson<TagRef[]>(`/api/notes/${noteId}/tags`, 'POST', { title }),
  removeNoteTag: (noteId: string, tagId: string) =>
    sendJson<TagRef[]>(`/api/notes/${noteId}/tags/${encodeURIComponent(tagId)}`, 'DELETE', undefined),
  resourceUrl: (id: string) => `/api/resources/${id}`,

  // 写入
  updateNote: (id: string, data: { title: string; body: string }) =>
    sendJson<NoteDetail>(`/api/notes/${id}`, 'PUT', data),
  // 移动笔记到另一个笔记本（改 parent_id）
  moveNote: (id: string, parentId: string) =>
    sendJson<NoteDetail>(`/api/notes/${id}/move`, 'PUT', { parent_id: parentId }),
  createNote: (data: { parent_id: string; title?: string; body?: string; is_todo?: boolean }) =>
    sendJson<NoteDetail>('/api/notes', 'POST', data),
  deleteNote: async (id: string) => {
    const res = await fetch(`/api/notes/${id}`, { method: 'DELETE', headers: authHeaders() })
    if (!res.ok) throw new Error(`DELETE -> ${res.status}`)
  },

  // 上传资源（图片/附件）：原始二进制作请求体，Content-Type=文件 MIME，文件名走 query。
  uploadResource: async (file: Blob, filename: string): Promise<ResourceUpload> => {
    const res = await fetch(`/api/resources?filename=${encodeURIComponent(filename)}`, {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': file.type || 'application/octet-stream' }),
      body: file,
    })
    if (!res.ok) throw new Error(`${t('api.uploadFailed')} -> ${res.status}`)
    return res.json() as Promise<ResourceUpload>
  },

  // 资源管理
  resources: () => getJson<ResourceInfo[]>('/api/resources'),
  renameResource: (id: string, title: string) =>
    sendJson<ResourceInfo>(`/api/resources/${id}`, 'PUT', { title }),
  deleteResource: async (id: string) => {
    const res = await fetch(`/api/resources/${id}`, { method: 'DELETE', headers: authHeaders() })
    if (!res.ok) throw new Error(`${t('api.deleteResFailed')} -> ${res.status}`)
  },

  // ---------- 插件 ----------
  // 探测坑：服务端未编译 plugins feature 时路由不存在，但 SPA fallback 会对
  // GET /api/plugins 回 200 的 index.html —— 必须校验 content-type 才能判定不可用。
  plugins: async (): Promise<PluginsResp | null> => {
    try {
      const res = await fetch('/api/plugins')
      const ct = res.headers.get('content-type') ?? ''
      if (!res.ok || !ct.includes('application/json')) return null
      return (await res.json()) as PluginsResp
    } catch {
      return null
    }
  },
  // 安装 .jplug/.zip：原始二进制作请求体（同资源上传惯例）。失败抛带服务端 message 的 Error。
  installPlugin: async (file: Blob, force = false): Promise<PluginInstallResult> => {
    const res = await fetch(`/api/plugins/install${force ? '?force=true' : ''}`, {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/zip' }),
      body: file,
    })
    const body = (await res.json().catch(() => null)) as
      | (PluginInstallResult & { error?: string; message?: string })
      | null
    if (!res.ok) throw new Error(body?.message || body?.error || `install -> ${res.status}`)
    return body as PluginInstallResult
  },
  deletePlugin: async (id: string) => {
    const res = await fetch(`/api/plugins/${id}`, { method: 'DELETE', headers: authHeaders() })
    if (!res.ok) {
      const body = (await res.json().catch(() => null)) as { error?: string; message?: string } | null
      throw new Error(body?.message || body?.error || `DELETE plugin -> ${res.status}`)
    }
  },
  setPluginEnabled: (id: string, enabled: boolean) =>
    sendJson<PluginInfo>(`/api/plugins/${id}/enable`, 'POST', { enabled }),
  pluginSettings: (id: string) => getJson<PluginSettingsResp>(`/api/plugins/${id}/settings`),
  savePluginSettings: async (id: string, values: Record<string, unknown>) => {
    const res = await fetch(`/api/plugins/${id}/settings`, {
      method: 'PUT',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify({ values }),
    })
    if (!res.ok) throw new Error(`PUT plugin settings -> ${res.status}`)
  },
  // 插件资产 URL（主题 css 等）；?v=版本 破 no-cache 后的旧缓存
  pluginAssetUrl: (id: string, path: string, version: string) =>
    `/api/plugins/${id}/assets/${path}?v=${encodeURIComponent(version)}`,
  // 执行插件 backend 命令（spec §9.4）。失败抛带服务端 message 的 Error。
  // 0.3 起返回完整响应（result + pending_writes）——调用方须把 pending_writes 交给确认队列。
  runPluginCommand: async (
    pluginId: string,
    commandId: string,
    args: Record<string, unknown>,
  ): Promise<PluginCommandResp> => {
    const res = await fetch(`/api/plugins/${pluginId}/commands/${commandId}`, {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify({ args }),
    })
    const body = (await res.json().catch(() => null)) as
      | (Partial<PluginCommandResp> & { error?: string; message?: string })
      | null
    if (!res.ok) throw new Error(body?.message || body?.error || `command -> ${res.status}`)
    return { result: body?.result ?? {}, pending_writes: body?.pending_writes ?? [] }
  },
  // server-driven UI：取插件某 view 的声明树（spec §9.5）。
  pluginUi: async (pluginId: string, view: string, state: unknown = null): Promise<PluginUiResp> => {
    const res = await fetch(`/api/plugins/${pluginId}/ui/${view}`, {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify({ state }),
    })
    const body = (await res.json().catch(() => null)) as
      | (Partial<PluginUiResp> & { error?: string; message?: string })
      | null
    if (!res.ok || !body?.ui) throw new Error(body?.message || body?.error || `ui -> ${res.status}`)
    return { ui: body.ui, pending_writes: body.pending_writes ?? [] }
  },
  // 编辑期文本变换（spec §3.7/§6.5，0.4 阶段 4）：把编辑缓冲文本交给插件 editor.transform，
  // 返回变换后的文本。纯文本 in/out，无 pending_writes。失败抛带服务端 message 的 Error。
  editorTransform: async (
    pluginId: string,
    phase: 'before-save' | 'input',
    text: string,
  ): Promise<string> => {
    const res = await fetch(`/api/plugins/${pluginId}/editor/transform`, {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify({ phase, text }),
    })
    const body = (await res.json().catch(() => null)) as
      | { text?: string; error?: string; message?: string }
      | null
    if (!res.ok || typeof body?.text !== 'string')
      throw new Error(body?.message || body?.error || `editor.transform -> ${res.status}`)
    return body.text
  },
  // notes:write 的「写入免确认」开关（宿主托管，spec §9.5）。
  setPluginAutoApprove: (id: string, enabled: boolean) =>
    sendJson<PluginInfo>(`/api/plugins/${id}/auto-approve`, 'PUT', { enabled }),
  // 持久化当前 UI 语言到服务端（供插件经 host_call system.locale 读取）。尽力而为：
  // 只读/未授权时会 4xx——静默忽略（前端展示仍由 localStorage 驱动）。
  putLocale: async (locale: string): Promise<void> => {
    try {
      await fetch('/api/locale', {
        method: 'PUT',
        headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ locale }),
      })
    } catch {
      /* 网络失败忽略 */
    }
  },
  // 宿主级 AI 配置（api_key 回显，与数据源密码同姿势）。
  getAiConfig: () => getJson<AiConfig>('/api/ai/config'),
  saveAiConfig: async (cfg: AiConfig) => {
    const res = await fetch('/api/ai/config', {
      method: 'PUT',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify(cfg),
    })
    if (!res.ok) {
      const body = (await res.json().catch(() => null)) as { error?: string; message?: string } | null
      throw new Error(body?.message || body?.error || `PUT ai config -> ${res.status}`)
    }
  },
}

// demo 模式只覆盖只读路径；写入/资源等仍是 httpApi（demo 站点里不会触发）。
const demoApi = {
  status: async (): Promise<StatusResp> => ({
    configured: true,
    source_type: 'demo',
    notes: 0,
    folders: 0,
    read_only: true, // demo 天然只读
    auth_enabled: false, // demo 无鉴权
    authenticated: false,
    passwordless_read: false,
    version: '0.0.0-demo',
  }),
  folders: async (): Promise<FolderNode[]> => JSON.parse((await wasmDemo()).folders()),
  tags: async (): Promise<TagInfo[]> => JSON.parse((await wasmDemo()).tags()),
  notesByTag: async (tagId: string): Promise<NoteSummary[]> =>
    JSON.parse((await wasmDemo()).notesByTag(tagId)),
  notes: async (folderId: string): Promise<NoteSummary[]> =>
    JSON.parse((await wasmDemo()).notes(folderId)),
  note: async (id: string): Promise<NoteDetail> => JSON.parse((await wasmDemo()).note(id)),
  search: async (q: string): Promise<NoteSummary[]> => JSON.parse((await wasmDemo()).search(q)),
}

// ---------- 可写本地应用（WASM_WRITABLE）：可插拔持久后端 ----------
// WASM 内核 + now 由 JS 注入 Date.now()（wasm 无时钟）。持久后端运行时二选一：
// - idb：浏览器 IndexedDB（默认；首次播种演示库）。
// - fsa：真实 Joplin 文件夹（Chromium，File System Access）；读写落磁盘、Joplin 可同步。

const resourceUrls = new Map<string, string>() // 资源 id → blob objectURL（resourceUrl 同步查表）

function genLocalId(): string {
  const b = new Uint8Array(16)
  crypto.getRandomValues(b)
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('')
}
function extOfFilename(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 && i < name.length - 1 ? name.slice(i + 1).toLowerCase() : ''
}
// 从原始 .md 取条目 id（FSA 后端 per-file 增量写用）。
function idOfRaw(raw: string): string | null {
  const m = /^id: ([0-9a-zA-Z]{32})$/m.exec(raw)
  return m ? m[1] : null
}
function snapshotMap(inst: WasmInstance): Map<string, string> {
  const m = new Map<string, string>()
  for (const raw of inst.snapshot()) {
    const id = idOfRaw(raw)
    if (id) m.set(id, raw)
  }
  return m
}

// 持久后端抽象：读全部原始条目 + 持久快照 + 资源二进制读写。
interface LocalBackend {
  isFolder: boolean
  label: string // 数据源名（文件夹名 / 'browser'）
  loadRaws(): Promise<string[]>
  persist(inst: WasmInstance): Promise<void>
  putResource(id: string, mime: string, blob: Blob): Promise<void>
  deleteResource(id: string): Promise<void>
  resourceBlob(id: string): Promise<Blob | null>
  allResourceIds(): Promise<string[]>
}

// IndexedDB 后端（默认）。
const idbBackend: LocalBackend = {
  isFolder: false,
  label: 'browser',
  loadRaws: async () => {
    let raws = await localStore.loadRaws()
    if (raws === null && !(await localStore.isSeeded())) {
      const m = await loadWasm()
      raws = m.Demo.seedItems() // 首次运行播种演示库（用户可自行清空）
      await localStore.saveRaws(raws)
      await localStore.markSeeded()
    }
    return raws ?? []
  },
  persist: async (inst) => {
    await localStore.saveRaws(inst.snapshot()) // 全量快照
  },
  putResource: (id, mime, blob) => localStore.putResource(id, mime, blob),
  deleteResource: (id) => localStore.deleteResource(id),
  resourceBlob: async (id) => (await localStore.getResource(id))?.blob ?? null,
  allResourceIds: async () => (await localStore.allResources()).map((r) => r.id),
}

// FSA 真实文件夹后端。per-file 增量写：diff 快照，只写改动、删已移除的 <id>.md。
let prevSnapshot = new Map<string, string>()
function fsaBackend(handle: FileSystemDirectoryHandle): LocalBackend {
  return {
    isFolder: true,
    label: handle.name,
    loadRaws: async () => {
      const raws = await fsStore.readAllItems(handle)
      prevSnapshot = snapshotOfRaws(raws)
      return raws
    },
    persist: async (inst) => {
      const cur = snapshotMap(inst)
      for (const [id, raw] of cur) {
        if (prevSnapshot.get(id) !== raw) await fsStore.writeItem(handle, id, raw)
      }
      for (const id of prevSnapshot.keys()) {
        if (!cur.has(id)) await fsStore.deleteItemFile(handle, id)
      }
      prevSnapshot = cur
    },
    putResource: (id, _mime, blob) => fsStore.writeResource(handle, id, blob),
    deleteResource: (id) => fsStore.deleteResourceFile(handle, id),
    resourceBlob: (id) => fsStore.readResourceBlob(handle, id),
    allResourceIds: () => fsStore.listResourceIds(handle),
  }
}
function snapshotOfRaws(raws: string[]): Map<string, string> {
  const m = new Map<string, string>()
  for (const raw of raws) {
    const id = idOfRaw(raw)
    if (id) m.set(id, raw)
  }
  return m
}

let backend: LocalBackend = idbBackend
let _local: Promise<WasmInstance> | null = null

// 用当前后端构建 wasm 实例 + 重建资源 blob URL 映射。
async function buildInstance(): Promise<WasmInstance> {
  const m = await loadWasm()
  const inst = m.Demo.fromRaws(await backend.loadRaws())
  for (const url of resourceUrls.values()) URL.revokeObjectURL(url)
  resourceUrls.clear()
  for (const id of await backend.allResourceIds()) {
    const blob = await backend.resourceBlob(id)
    if (blob) resourceUrls.set(id, URL.createObjectURL(blob))
  }
  return inst
}

function localInst(): Promise<WasmInstance> {
  if (!_local) {
    _local = (async () => {
      // 启动优先恢复「已授权」的文件夹（权限还在才自动用；否则退回浏览器存储，UI 可提示重连）。
      if (fsStore.fsaSupported()) {
        try {
          const h = await fsStore.loadHandle()
          if (h && (await fsStore.ensurePermission(h, false))) backend = fsaBackend(h)
        } catch {
          /* 退回 idb */
        }
      }
      return buildInstance()
    })()
  }
  return _local
}
async function persistLocal(inst: WasmInstance): Promise<void> {
  await backend.persist(inst)
}

// 切换后端后重建实例（下次 api 调用即用新库）。
async function switchBackend(next: LocalBackend): Promise<void> {
  backend = next
  _local = buildInstance()
  await _local
}

// ---- 真实文件夹控制（挂到 localFolder 导出；native 构建里整体 tree-shake）----
async function openLocalFolder(): Promise<string> {
  const h = await fsStore.pickDirectory() // 用户取消 → reject
  if (!(await fsStore.ensurePermission(h, true))) throw new Error('folder permission denied')
  await fsStore.saveHandle(h)
  await switchBackend(fsaBackend(h))
  return h.name
}
async function reconnectLocalFolder(): Promise<string | null> {
  const h = await fsStore.loadHandle()
  if (!h || !(await fsStore.ensurePermission(h, true))) return null
  await fsStore.saveHandle(h)
  await switchBackend(fsaBackend(h))
  return h.name
}
async function closeLocalFolder(): Promise<void> {
  await fsStore.clearHandle()
  await switchBackend(idbBackend)
}
// 有已保存但需重新授权的文件夹句柄（回访权限过期）→ 名字；否则 null。
async function pendingLocalFolder(): Promise<string | null> {
  if (backend.isFolder) return null
  try {
    const h = await fsStore.loadHandle()
    if (h && !(await fsStore.ensurePermission(h, false))) return h.name
  } catch {
    /* 无 */
  }
  return null
}

const localApi = {
  status: async (): Promise<StatusResp> => ({
    configured: true,
    source_type: 'local-wasm',
    notes: 0,
    folders: 0,
    read_only: false, // 本地应用可写
    auth_enabled: false,
    authenticated: false,
    passwordless_read: false,
    version: '0.0.0-local',
  }),
  folders: async (): Promise<FolderNode[]> => JSON.parse((await localInst()).folders()),
  notes: async (folderId: string): Promise<NoteSummary[]> =>
    JSON.parse((await localInst()).notes(folderId)),
  note: async (id: string): Promise<NoteDetail> => JSON.parse((await localInst()).note(id)),
  search: async (q: string): Promise<NoteSummary[]> => JSON.parse((await localInst()).search(q)),
  tags: async (): Promise<TagInfo[]> => JSON.parse((await localInst()).tags()),
  notesByTag: async (tagId: string): Promise<NoteSummary[]> =>
    JSON.parse((await localInst()).notesByTag(tagId)),
  noteTags: async (noteId: string): Promise<TagRef[]> =>
    JSON.parse((await localInst()).noteTags(noteId)),
  resourceUrl: (id: string): string => resourceUrls.get(id) ?? '',

  updateNote: async (id: string, data: { title: string; body: string }): Promise<NoteDetail> => {
    const i = await localInst()
    const d = JSON.parse(i.updateNote(id, data.title, data.body, Date.now())) as NoteDetail
    await persistLocal(i)
    return d
  },
  createNote: async (data: {
    parent_id: string
    title?: string
    body?: string
    is_todo?: boolean
  }): Promise<NoteDetail> => {
    const i = await localInst()
    const d = JSON.parse(
      i.createNote(data.parent_id, data.title ?? '', data.body ?? '', !!data.is_todo, Date.now()),
    ) as NoteDetail
    await persistLocal(i)
    return d
  },
  moveNote: async (id: string, parentId: string): Promise<NoteDetail> => {
    const i = await localInst()
    const d = JSON.parse(i.moveNote(id, parentId, Date.now())) as NoteDetail
    await persistLocal(i)
    return d
  },
  deleteNote: async (id: string): Promise<void> => {
    const i = await localInst()
    i.deleteNote(id)
    await persistLocal(i)
  },
  createFolder: async (data: { parent_id: string; title: string }): Promise<FolderRef> => {
    const i = await localInst()
    const f = JSON.parse(i.createFolder(data.parent_id, data.title, Date.now())) as FolderRef
    await persistLocal(i)
    return f
  },
  renameFolder: async (id: string, title: string): Promise<FolderRef> => {
    const i = await localInst()
    const f = JSON.parse(i.renameFolder(id, title, Date.now())) as FolderRef
    await persistLocal(i)
    return f
  },
  moveFolder: async (id: string, parentId: string): Promise<FolderRef> => {
    const i = await localInst()
    const f = JSON.parse(i.moveFolder(id, parentId, Date.now())) as FolderRef
    await persistLocal(i)
    return f
  },
  deleteFolder: async (id: string): Promise<void> => {
    const i = await localInst()
    i.deleteFolder(id)
    await persistLocal(i)
  },
  addNoteTag: async (noteId: string, title: string): Promise<TagRef[]> => {
    const i = await localInst()
    const tags = JSON.parse(i.addNoteTag(noteId, title, Date.now())) as TagRef[]
    await persistLocal(i)
    return tags
  },
  removeNoteTag: async (noteId: string, tagId: string): Promise<TagRef[]> => {
    const i = await localInst()
    const tags = JSON.parse(i.removeNoteTag(noteId, tagId)) as TagRef[]
    await persistLocal(i)
    return tags
  },

  resources: async (): Promise<ResourceInfo[]> => JSON.parse((await localInst()).resources()),
  uploadResource: async (file: Blob, filename: string): Promise<ResourceUpload> => {
    const i = await localInst()
    const id = genLocalId()
    const mime = file.type || 'application/octet-stream'
    const name = filename.trim()
    const ext = extOfFilename(name)
    const title = name || (ext ? `${id}.${ext}` : id)
    await backend.putResource(id, mime, file)
    resourceUrls.set(id, URL.createObjectURL(file))
    const up = JSON.parse(
      i.upsertResourceMeta(id, title, mime, ext, file.size, Date.now()),
    ) as ResourceUpload
    await persistLocal(i)
    return up
  },
  renameResource: async (id: string, title: string): Promise<ResourceInfo> => {
    const i = await localInst()
    const r = JSON.parse(i.renameResource(id, title, Date.now())) as ResourceInfo
    await persistLocal(i)
    return r
  },
  deleteResource: async (id: string): Promise<void> => {
    const i = await localInst()
    i.deleteResource(id)
    const url = resourceUrls.get(id)
    if (url) {
      URL.revokeObjectURL(url)
      resourceUrls.delete(id)
    }
    await backend.deleteResource(id)
    await persistLocal(i)
  },
}

// 真实文件夹（FSA）控制。仅本地可写构建暴露；native 构建 WASM_WRITABLE 折叠为常量 false
// → localFolder=undefined、控制函数/fsStore/backend 一并 tree-shake，不把 wasm 拖进原生包。
const localFolderControls = {
  supported: (): boolean => fsStore.fsaSupported(),
  name: (): string | null => (backend.isFolder ? backend.label : null),
  open: openLocalFolder,
  reconnect: reconnectLocalFolder,
  close: closeLocalFolder,
  pending: pendingLocalFolder,
}
export const localFolder: typeof localFolderControls | undefined = WASM_WRITABLE
  ? localFolderControls
  : undefined

export const api = WASM_WRITABLE
  ? { ...httpApi, ...localApi }
  : DEMO
    ? { ...httpApi, ...demoApi }
    : httpApi
