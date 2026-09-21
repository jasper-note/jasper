<script lang="ts">
  import { onMount } from 'svelte'
  import { fade, slide } from 'svelte/transition'
  import {
    api,
    localFolder,
    IS_DEMO,
    IS_WASM,
    FOLDER_DND_TYPE,
    setAuthErrorHandler,
    type FolderNode,
    type NoteSummary,
    type NoteDetail,
    type PendingWrite,
    type StatusResp,
    type TagInfo,
  } from './lib/api'
  import AuthDialog from './lib/AuthDialog.svelte'
  import { draggingFolder } from './lib/dnd.svelte'
  import { t, getLocale } from './lib/i18n.svelte'
  import Button from './lib/Button.svelte'
  import Icon from './lib/Icon.svelte'
  import ThemePicker from './lib/ThemePicker.svelte'
  import LangPicker from './lib/LangPicker.svelte'
  import FolderTree from './lib/FolderTree.svelte'
  import TagList from './lib/TagList.svelte'
  import NoteList from './lib/NoteList.svelte'
  import NoteView from './lib/NoteView.svelte'
  import PdfViewer from './lib/PdfViewer.svelte'
  import { loadResourceMeta } from './lib/resourceMeta.svelte'
  import Settings from './lib/Settings.svelte'
  import ResourcePanel from './lib/ResourcePanel.svelte'
  import PluginPanel from './lib/PluginPanel.svelte'
  import PluginSidebar from './lib/PluginSidebar.svelte'
  import PendingWriteDialog from './lib/PendingWriteDialog.svelte'
  import { loadPlugins, pluginsAvailable, sidebarContributions, type SidebarEntry } from './lib/plugins.svelte'
  import { installSelectionCapture, clearSelection } from './lib/selection.svelte'
  import { connectEvents, type ChangeEvent } from './lib/events'

  // 让 <html lang> 跟随当前语言（影响断词/无障碍等），并把当前语言持久化到服务端
  // （启动 + 每次切换）——插件经 host_call system.locale 读「系统语言」用同一值。
  $effect(() => {
    const l = getLocale()
    document.documentElement.lang = l
    if (!IS_WASM) api.putLocale(l)
  })


  let folders = $state<FolderNode[]>([])
  let selectedFolderId = $state<string | null>(null)
  // 标签视图（只读浏览，与笔记本选择互斥）：选中某标签时 selectedFolderId=null
  let tags = $state<TagInfo[]>([])
  let selectedTagId = $state<string | null>(null)
  // 打开中的笔记标签行的外部刷新令牌（SSE tag 事件命中该笔记时自增 → NoteTags 重载）
  let tagRefreshToken = $state(0)
  let notes = $state<NoteSummary[]>([])
  let selectedNoteId = $state<string | null>(null)
  let detail = $state<NoteDetail | null>(null)
  // NoteView 组件实例（bind:this）：SSE 外部变更经 applyExternal 保守回显
  let noteView = $state<ReturnType<typeof NoteView> | null>(null)

  let query = $state('')
  let searchMode = $state(false)
  let listTitle = $state('')
  let error = $state('')

  let configured = $state<boolean | null>(null)
  let showSettings = $state(false)
  let showResources = $state(false)
  let showPlugins = $state(false)
  let showDemoBanner = $state(true)
  // 本地可写应用的真实文件夹数据源（FSA，仅 Chromium）：当前打开的文件夹名 / 待重连的文件夹名
  let folderName = $state<string | null>(null)
  let pendingFolder = $state<string | null>(null)
  // 首次提示：Chrome 用户第一次进来时，在「打开文件夹」按钮下方悬浮一次性 tip（可离线选本地文件夹）
  let showFolderHint = $state(false)
  const FS_HINT_KEY = 'jasper.fsHintSeen'
  function fsHintSeen(): boolean {
    try {
      return localStorage.getItem(FS_HINT_KEY) === '1'
    } catch {
      return false
    }
  }
  function dismissFolderHint() {
    try {
      localStorage.setItem(FS_HINT_KEY, '1')
    } catch {
      /* localStorage 不可用 → 仅本次隐藏 */
    }
    showFolderHint = false
  }

  // 访问鉴权（access control）状态（来自 /api/status）
  let authEnabled = $state(false) // 是否设了访问密码
  let authenticated = $state(false) // 本会话是否已登录
  let passwordlessRead = $state(false) // 允许无密码阅读
  let showAuthDialog = $state(false) // 登录框开关
  // 受保护但未登录：写入闸门收紧为只读（下方 readOnly 合并）；且当无可见内容时给登录提示
  const locked = $derived(authEnabled && !authenticated)

  // 插件侧边栏（右侧 dock，spec §3.5/§9.4）：当前打开的面板；插件被禁用/卸载后自动关闭
  let dockEntry = $state<SidebarEntry | null>(null)
  $effect(() => {
    if (
      dockEntry &&
      !sidebarContributions().some(
        (e) => e.pluginId === dockEntry!.pluginId && e.contribution.id === dockEntry!.contribution.id,
      )
    ) {
      dockEntry = null
    }
  })
  function toggleDock(entry: SidebarEntry) {
    dockEntry =
      dockEntry?.pluginId === entry.pluginId && dockEntry.contribution.id === entry.contribution.id
        ? null
        : entry
  }

  // 插件写提案获批落盘后：受影响的打开笔记强制重挂载取新内容，列表/树跟着刷新
  async function onPluginWriteApplied(w: PendingWrite) {
    try {
      if (w.action === 'update' && selectedNoteId === w.note.id) {
        const id = w.note.id
        selectedNoteId = null
        detail = null
        await selectNote(id)
      }
      if (w.action === 'create') folders = await api.folders()
      await refreshList()
    } catch {
      /* 忽略 */
    }
  }
  // 服务端只读模式（/api/status 返回）。与编译期 demo 只读、未登录锁定合并成统一的写入闸门。
  let serverReadOnly = $state(false)
  const readOnly = $derived(IS_DEMO || serverReadOnly || locked)

  // 把一次 /api/status 的开关同步进本地状态（checkStatus / onConfigured / 登录后共用）。
  function applyStatusFlags(s: StatusResp) {
    serverReadOnly = s.read_only
    authEnabled = s.auth_enabled
    authenticated = s.authenticated
    passwordlessRead = s.passwordless_read
  }

  // 登录成功：关闭登录框并整体重载（现在可写 + 可见全部）
  async function onLoginSuccess() {
    showAuthDialog = false
    await checkStatus()
  }
  // 登出：吊销会话 + 清 token → 重载（回到匿名可见范围）
  async function doLogout() {
    try {
      await api.logout()
    } catch {
      /* 忽略 */
    }
    await checkStatus()
  }

  // 访问控制设置在设置页被改动后：刷新状态闸门（不重载数据、不关设置页）
  async function refreshStatus() {
    try {
      applyStatusFlags(await api.status())
    } catch {
      /* 忽略 */
    }
  }

  // ---- SSE 变更 → 去抖合并刷新（/api/events）----
  // 自己的写入也会回声：列表刷新幂等、打开中的笔记走 NoteView.applyExternal 的
  // §5.3 保守规则（内容相同/正在输入都不动缓冲），所以无需区分事件来源。
  const remotePending = {
    folders: false,
    list: false,
    openNote: false,
    openNoteDeleted: false,
    tags: false,
    openNoteTags: false,
  }
  let remoteTimer: ReturnType<typeof setTimeout> | undefined

  function onRemoteChange(ev: ChangeEvent) {
    if (ev.kind === 'library') {
      // 整库替换（数据源切换/服务重启后重连/受限匿名的粗粒度事件）：全量刷新，但保留当前选中与
      // 打开的笔记，见 reloadLibrary()
      clearTimeout(remoteTimer)
      remotePending.folders = remotePending.list = remotePending.openNote = false
      remotePending.openNoteDeleted = remotePending.tags = remotePending.openNoteTags = false
      void reloadLibrary()
      return
    }
    if (ev.kind === 'folder') {
      remotePending.folders = remotePending.list = true
    } else if (ev.kind === 'tag') {
      // 某笔记的标签集变化（id 为该笔记）：刷侧栏标签区；若在看该标签则刷列表；命中打开的笔记则刷其标签行
      remotePending.tags = true
      if (selectedTagId) remotePending.list = true
      if (ev.id === selectedNoteId) remotePending.openNoteTags = true
    } else {
      remotePending.list = true
      // 未知 id 的 upsert 可能是新建/移动（笔记本计数变了）；删除同理
      const known = notes.some((n) => n.id === ev.id)
      if (ev.op === 'delete' || !known) remotePending.folders = true
      if (ev.id === selectedNoteId) {
        if (ev.op === 'delete') remotePending.openNoteDeleted = true
        else remotePending.openNote = true
      }
    }
    clearTimeout(remoteTimer)
    remoteTimer = setTimeout(applyRemoteChanges, 250)
  }

  async function applyRemoteChanges() {
    const { folders: doFolders, openNote, openNoteDeleted } = remotePending
    let doList = remotePending.list
    const { tags: doTags, openNoteTags } = remotePending
    remotePending.folders = remotePending.list = remotePending.openNote = false
    remotePending.openNoteDeleted = remotePending.tags = remotePending.openNoteTags = false
    try {
      if (openNoteDeleted) {
        // 打开中的笔记被外部删除：关视图（未保存输入已无处可保存）
        selectedNoteId = null
        detail = null
        saveLastNoteId(null)
      }
      if (doFolders) {
        folders = await api.folders()
        // 当前选中的笔记本已不在树上（被外部删除 / 级联删掉）→ 退回未选中态，
        // 否则接下来的 refreshList 会去拉一个不存在的笔记本。
        if (selectedFolderId != null && !folderExists(folders, selectedFolderId)) {
          selectedFolderId = null
          saveLastFolderId(null)
          listTitle = ''
          notes = []
          doList = false
        }
      }
      // 标签篇数随笔记增删/打标签变化 → 侧栏标签区随笔记本树或 tag 事件刷新
      if (doFolders || doTags) void loadTags()
      if (openNoteTags) tagRefreshToken++ // 打开的笔记标签被外部改动 → NoteTags 重载
      if (doList) await refreshList()
      if (openNote && !openNoteDeleted && selectedNoteId) {
        const fresh = await api.note(selectedNoteId)
        if (noteView?.applyExternal(fresh)) detail = fresh
      }
    } catch {
      /* 网络抖动忽略；下一个事件会再触发 */
    }
  }

  // library reload（数据源切换/服务重启后重连/受限匿名收到的粗粒度事件）：全量刷新树、标签、
  // 列表，并把打开的笔记按 §5.3 保守规则（NoteView.applyExternal）对齐服务端——
  // **保留 selectedNoteId，不重挂载 NoteView**。
  // 旧实现直接调 checkStatus() → loadFolders() 会先把 selectedNoteId/detail 清空再恢复，
  // 于是每收到一条 library reload 就销毁重建一次 CodeMirror：编辑区闪烁、光标丢失。
  async function reloadLibrary() {
    try {
      applyStatusFlags(await api.status())
    } catch {
      /* 忽略；下一个事件会再触发 */
    }
    // 还没有任何选中（刚进来/上次恢复失败）→ 没什么可保的，走完整恢复流程
    if (selectedNoteId == null && selectedFolderId == null && selectedTagId == null && !searchMode) {
      await loadFolders()
      return
    }
    try {
      folders = await api.folders()
      void loadTags()
      // 正在看的笔记本已不存在（换了数据源/被删）→ 只有这种情况才整体重载回默认视图
      const folderGone =
        !searchMode &&
        selectedTagId == null &&
        selectedFolderId != null &&
        selectedFolderId !== '' &&
        !folderExists(folders, selectedFolderId)
      if (folderGone) {
        await loadFolders()
        return
      }
      await refreshList()
      if (selectedNoteId) {
        const fresh = await api.note(selectedNoteId)
        tagRefreshToken++ // 标签行也可能变了
        if (noteView?.applyExternal(fresh)) detail = fresh
      }
    } catch {
      /* 网络抖动/笔记已不可见：保持当前视图，绝不丢用户未保存的输入 */
    }
  }

  // 本地给当前笔记打/去标签后：刷新侧栏标签区（含篇数）；若正在看某标签则刷新其笔记列表
  async function onNoteTagsChanged() {
    void loadTags()
    if (selectedTagId) await refreshList()
  }

  // 资源被删除/重命名后，刷新当前笔记详情（被引用资源变动可能影响渲染）
  async function onResourcesChanged() {
    if (selectedNoteId) {
      try {
        detail = await api.note(selectedNoteId)
      } catch {
        /* 忽略 */
      }
    }
  }

  // 记忆上次打开的笔记：仅存笔记 id，重载时按 id 拉取（失效则回退）。
  const LAST_NOTE_KEY = 'jasper.lastNote'
  function loadLastNoteId(): string | null {
    try {
      return localStorage.getItem(LAST_NOTE_KEY)
    } catch {
      return null
    }
  }
  function saveLastNoteId(id: string | null) {
    try {
      if (id) localStorage.setItem(LAST_NOTE_KEY, id)
      else localStorage.removeItem(LAST_NOTE_KEY)
    } catch {
      /* 忽略（隐私模式/storage 被禁用） */
    }
  }

  // 记忆上次选中的笔记本（列表视图）：与上次打开的笔记(lastNote)正交——笔记本决定左侧列表、
  // 笔记决定详情面板，二者可不同源（搜索/内链打开的笔记未必在当前笔记本）。
  // 空串 "" = 未分类笔记（合法 id）：localStorage 无键(null)=无记忆，需与 "" 区分。
  const LAST_FOLDER_KEY = 'jasper.lastFolder'
  function loadLastFolderId(): string | null {
    try {
      return localStorage.getItem(LAST_FOLDER_KEY) // null=无记忆；""=未分类；其它=笔记本 id
    } catch {
      return null
    }
  }
  function saveLastFolderId(id: string | null) {
    try {
      if (id === null) localStorage.removeItem(LAST_FOLDER_KEY)
      else localStorage.setItem(LAST_FOLDER_KEY, id) // 允许存 ""（未分类）
    } catch {
      /* 忽略（隐私模式/storage 被禁用） */
    }
  }
  // 递归判断某笔记本仍存在（记忆值可能指向已被删除/移动源的笔记本）。
  function folderExists(list: FolderNode[], id: string): boolean {
    for (const f of list) {
      if (f.id === id) return true
      if (folderExists(f.children, id)) return true
    }
    return false
  }

  onMount(() => {
    // token 失效（服务端重启/改密码使会话作废）→ 回退只读并重新拉状态
    setAuthErrorHandler(() => {
      authenticated = false
      void checkStatus()
    })
    void checkStatus()
    // 捕获笔记内容区的文字选区（供插件侧栏把「选中的文字」并入命令 args）
    installSelectionCapture()
  })

  // 切换打开的笔记 → 清掉上一篇的陈旧选区
  $effect(() => {
    void selectedNoteId
    clearSelection()
  })

  async function checkStatus() {
    // 插件列表并行加载（探测服务端是否编译 plugins feature；注入插件主题 CSS）
    void loadPlugins()
    // 资源元数据（id→mime）：决定 :/id 渲染成 图片/视频/音频/PDF卡片/文件卡片
    void loadResourceMeta()
    try {
      const s = await api.status()
      configured = s.configured
      applyStatusFlags(s)
      if (configured) {
        connectEvents(onRemoteChange) // 幂等；DEMO/不支持时静默跳过
        await loadFolders()
        await refreshLocalSource()
      }
    } catch (e) {
      error = `${e}`
    }
  }

  // 本地可写应用：把当前数据源（浏览器存储 / 真实文件夹）同步进 UI。
  async function refreshLocalSource() {
    if (!localFolder) return
    folderName = localFolder.name()
    pendingFolder = folderName ? null : await localFolder.pending()
    // 支持真实文件夹、当前用浏览器存储、且从未看过提示 → 首次悬浮 tip
    showFolderHint = localFolder.supported() && !folderName && !pendingFolder && !fsHintSeen()
  }
  // 数据源切换后重载库（清选中 → 重新拉笔记本/标签/笔记）。
  async function reloadFromSource() {
    selectedNoteId = null
    detail = null
    selectedTagId = null
    await loadFolders()
    await refreshLocalSource()
  }
  async function openFolder() {
    try {
      await localFolder!.open()
      dismissFolderHint() // 打开过即视为已知，避免关闭文件夹后再弹
      await reloadFromSource()
    } catch (e) {
      // 用户取消目录选择器（AbortError）→ 静默；其它错误提示。
      if (!(e instanceof DOMException && e.name === 'AbortError')) error = `${e}`
    }
  }
  async function reconnectFolder() {
    try {
      if (await localFolder!.reconnect()) await reloadFromSource()
    } catch {
      /* 授权失败 → 保持浏览器存储 */
    }
  }
  async function closeFolder() {
    await localFolder!.close()
    await reloadFromSource()
  }

  async function loadTags() {
    try {
      tags = await api.tags()
    } catch {
      /* 忽略（无标签/网络抖动） */
    }
  }

  async function loadFolders() {
    try {
      folders = await api.folders()
      void loadTags() // 标签区与笔记本树并列，随之刷新（互不阻塞）
      selectedTagId = null // 整体重载回到笔记本视图
      detail = null
      selectedNoteId = null
      // 恢复上次会话：① 选中的笔记本（列表视图）② 打开的笔记（详情面板）——二者正交、各自持久。
      // 先恢复笔记本视图；再恢复笔记（若已恢复笔记本视图，则不让恢复笔记反过来改选笔记本）。
      const viewRestored = await restoreLastFolder()
      const noteRestored = await restoreLastNote({ selectFolder: !viewRestored })
      if (viewRestored || noteRestored) return
      const first = findFirstWithNotes(folders)
      if (first) {
        // 不传显式 title：合成的「未分类笔记」节点(id=="")后端留空标题，交给 findTitle 按语言取词
        selectFolder(first.id)
      } else {
        notes = []
        selectedFolderId = null
        listTitle = ''
      }
    } catch (e) {
      error = t('app.loadFailed', { err: `${e}` })
    }
  }

  // 恢复上次选中的笔记本（列表视图）；已删除/失效 → 清除记忆并放弃。
  async function restoreLastFolder(): Promise<boolean> {
    const id = loadLastFolderId()
    if (id == null) return false // 无记忆
    if (id !== '' && !folderExists(folders, id)) {
      saveLastFolderId(null)
      return false
    }
    await selectFolder(id)
    return true
  }

  // 恢复上次打开的笔记：拉取详情成功 → 打开；笔记已不存在 → 清除记忆并回退。
  // opts.selectFolder：是否顺带选中笔记所在笔记本（无「笔记本视图」记忆时为 true，保持旧行为）。
  async function restoreLastNote(opts: { selectFolder: boolean } = { selectFolder: true }): Promise<boolean> {
    const id = loadLastNoteId()
    if (!id) return false
    try {
      const d = await api.note(id)
      if (opts.selectFolder) await selectFolder(d.parent_id)
      detail = d
      selectedNoteId = id
      return true
    } catch {
      saveLastNoteId(null)
      return false
    }
  }

  // 配置完成（首次或切换数据源）后重载
  async function onConfigured() {
    configured = true
    showSettings = false
    query = ''
    searchMode = false
    // 只读/鉴权开关可能在设置里被切换 → 重新拉状态刷新写入闸门
    try {
      applyStatusFlags(await api.status())
    } catch {
      /* 忽略 */
    }
    await loadFolders()
  }

  function findFirstWithNotes(list: FolderNode[]): FolderNode | null {
    for (const f of list) {
      if (f.note_count > 0) return f
      const child = findFirstWithNotes(f.children)
      if (child) return child
    }
    return list[0] ?? null
  }

  function findTitle(list: FolderNode[], id: string): string {
    // 合成的「未分类笔记」节点（id==""）标题由后端留空，这里按当前语言取词。
    if (id === '') return t('tree.unfiled')
    for (const f of list) {
      if (f.id === id) return f.title
      const child = findTitle(f.children, id)
      if (child) return child
    }
    return ''
  }

  async function selectFolder(id: string, title?: string) {
    searchMode = false
    selectedTagId = null // 笔记本与标签选择互斥
    selectedFolderId = id
    saveLastFolderId(id) // 记忆当前笔记本，重载时恢复
    listTitle = title ?? findTitle(folders, id) ?? ''
    try {
      notes = await api.notes(id)
    } catch (e) {
      error = `${e}`
    }
  }

  // 选中某标签：列表显示打了该标签的笔记（互斥于笔记本选择）。
  async function selectTag(id: string, title: string) {
    searchMode = false
    selectedFolderId = null
    saveLastFolderId(null) // 切到标签视图 → 清除笔记本记忆，避免重载错误恢复旧笔记本
    selectedTagId = id
    listTitle = title
    try {
      notes = await api.notesByTag(id)
    } catch (e) {
      error = `${e}`
    }
  }

  // 先取详情再切换 id（NoteView 按 id 重挂载，挂载时 detail 须已就绪）
  async function selectNote(id: string) {
    try {
      const d = await api.note(id)
      detail = d
      selectedNoteId = id
      saveLastNoteId(id)
    } catch (e) {
      error = `${e}`
    }
  }

  // 笔记内部链接点击：是笔记则跳转，否则当资源在新标签打开
  async function navigate(id: string) {
    try {
      const d = await api.note(id)
      detail = d
      selectedNoteId = id
      saveLastNoteId(id)
    } catch {
      window.open(api.resourceUrl(id), '_blank')
    }
  }

  // PDF 阅读器模态（阅读视图/编辑器里的 PDF 卡片点击打开；自绘 pdf.js 阅读器）
  let pdfModal = $state<{ url: string; name: string } | null>(null)
  function openPdf(id: string, name: string) {
    pdfModal = { url: api.resourceUrl(id), name: name || 'PDF' }
  }

  // 重新拉取当前列表（保存后标题/时间/排序更新）
  async function refreshList() {
    try {
      if (searchMode) notes = await api.search(query.trim())
      else if (selectedTagId != null) notes = await api.notesByTag(selectedTagId)
      else if (selectedFolderId != null) notes = await api.notes(selectedFolderId)
    } catch (e) {
      error = `${e}`
    }
  }

  // 拖拽：把笔记移动到另一个笔记本（改 parent_id）
  async function moveNote(noteId: string, targetFolderId: string) {
    try {
      const d = await api.moveNote(noteId, targetFolderId)
      if (selectedNoteId === noteId) detail = d // 当前打开的就是它 → 同步详情
      folders = await api.folders() // 源/目标笔记本篇数变化
      await refreshList() // 移走的笔记从当前列表消失（剩余项 FLIP 平滑补位）
    } catch (e) {
      error = `${e}`
    }
  }

  // 拖拽：把笔记本移到另一个笔记本下（parentId 空=移到顶层）。后端防环。
  async function moveFolder(folderId: string, parentId: string) {
    try {
      await api.moveFolder(folderId, parentId)
      folders = await api.folders()
    } catch (e) {
      error = `${e}`
    }
  }

  // 重命名笔记本：名称走浏览器 prompt（预填当前名）。改完刷新树；若正是当前所选，同步列表标题。
  async function renameFolder(folderId: string, currentTitle: string) {
    const name = (prompt(t('notebook.renamePrompt'), currentTitle) ?? '').trim()
    if (!name || name === currentTitle) return
    try {
      await api.renameFolder(folderId, name)
      folders = await api.folders()
      if (!searchMode && selectedFolderId === folderId) listTitle = name
    } catch (e) {
      error = `${e}`
    }
  }

  // 删除笔记本：级联删掉整棵子树（子笔记本 + 其中全部笔记），不可撤销 → 先 confirm 报清条数。
  // 删完若当前选中的笔记本 / 打开的笔记正在这棵子树里，一并清掉（它们已经不存在了）。
  async function deleteFolder(folder: FolderNode) {
    const subtreeIds = new Set<string>()
    let subFolders = 0
    let noteCount = 0
    const walk = (f: FolderNode, isRoot: boolean) => {
      subtreeIds.add(f.id)
      if (!isRoot) subFolders++
      noteCount += f.note_count
      for (const c of f.children) walk(c, false)
    }
    walk(folder, true)

    const title = folder.title || t('common.unnamed')
    // 三档文案：空笔记本 / 只有笔记 / 还有子笔记本——避免出现「0 个子笔记本」这种别扭说法
    let msg: string
    if (subFolders + noteCount === 0) msg = t('notebook.confirmDeleteEmpty', { title })
    else if (subFolders === 0) msg = t('notebook.confirmDeleteNotes', { title, notes: noteCount })
    else msg = t('notebook.confirmDelete', { title, folders: subFolders, notes: noteCount })
    if (!confirm(msg)) return

    try {
      await api.deleteFolder(folder.id)
    } catch (e) {
      error = t('notebook.deleteFailed', { err: `${e}` })
      return
    }
    // 打开的笔记就在被删子树里 → 关掉（已无处可保存）
    if (detail && subtreeIds.has(detail.parent_id)) {
      detail = null
      selectedNoteId = null
      saveLastNoteId(null)
    }
    folders = await api.folders()
    if (selectedFolderId != null && subtreeIds.has(selectedFolderId)) {
      // 选中的笔记本没了 → 退回「未选中」态（列表清空），不擅自替用户选别的笔记本
      selectedFolderId = null
      saveLastFolderId(null)
      listTitle = ''
      notes = []
    } else {
      await refreshList()
    }
    void loadTags() // 笔记连同标签关联一起消失 → 侧栏标签篇数变了
  }

  // 新建笔记本（顶层）：名称走浏览器 prompt（带本地化默认名），建好后选中
  async function handleNewFolder() {
    const name = (prompt(t('notebook.namePrompt'), t('notebook.defaultName')) ?? '').trim()
    if (!name) return
    try {
      const f = await api.createFolder({ parent_id: '', title: name })
      folders = await api.folders()
      selectFolder(f.id, f.title)
    } catch (e) {
      error = `${e}`
    }
  }

  // 新建待办：与新建笔记同路径，仅 is_todo=true
  async function handleNewTodo() {
    const parent = searchMode ? '' : selectedFolderId ?? ''
    try {
      const n = await api.createNote({ parent_id: parent, title: t('note.newTodoTitle'), body: '', is_todo: true })
      detail = n
      selectedNoteId = n.id
      saveLastNoteId(n.id)
      // 之前未选中任何笔记本（如空白库）时，把选中态落到笔记实际所在的 parent，
      // 否则 refreshList 的 selectedFolderId!=null 判断会跳过拉取，新笔记不会出现在列表里。
      if (!searchMode && selectedFolderId == null) {
        selectedTagId = null // 标签视图下新建的是未打标签的笔记 → 落到未分类视图
        selectedFolderId = parent // parent 在此分支恒为 ''（未分类笔记）
        saveLastFolderId(parent) // 记忆随之落到未分类，保持与 selectedFolderId 一致
        listTitle = t('tree.unfiled')
      }
      folders = await api.folders() // 未分类笔记数/笔记本篇数变化
      await refreshList()
    } catch (e) {
      error = `${e}`
    }
  }

  // 「移到顶层」根放置区（仅拖拽笔记本时显示）
  let rootDropOver = $state(false)
  function onRootDragOver(e: DragEvent) {
    if (!e.dataTransfer?.types.includes(FOLDER_DND_TYPE)) return
    e.preventDefault()
    e.dataTransfer.dropEffect = 'move'
    rootDropOver = true
  }
  function onRootDrop(e: DragEvent) {
    rootDropOver = false
    const id = e.dataTransfer?.getData(FOLDER_DND_TYPE)
    if (id) moveFolder(id, '')
  }

  async function handleNew() {
    const parent = searchMode ? '' : selectedFolderId ?? ''
    try {
      const n = await api.createNote({ parent_id: parent, title: t('note.newNote'), body: '' })
      detail = n
      selectedNoteId = n.id
      saveLastNoteId(n.id)
      // 之前未选中任何笔记本（如空白库）时，把选中态落到笔记实际所在的 parent，
      // 否则 refreshList 的 selectedFolderId!=null 判断会跳过拉取，新笔记不会出现在列表里。
      if (!searchMode && selectedFolderId == null) {
        selectedTagId = null // 标签视图下新建的是未打标签的笔记 → 落到未分类视图
        selectedFolderId = parent // parent 在此分支恒为 ''（未分类笔记）
        saveLastFolderId(parent) // 记忆随之落到未分类，保持与 selectedFolderId 一致
        listTitle = t('tree.unfiled')
      }
      folders = await api.folders() // 未分类笔记数/笔记本篇数变化
      await refreshList()
    } catch (e) {
      error = `${e}`
    }
  }

  async function onNoteDeleted() {
    detail = null
    selectedNoteId = null
    saveLastNoteId(null)
    await refreshList()
  }

  let searchTimer: ReturnType<typeof setTimeout> | undefined
  function onSearchInput() {
    clearTimeout(searchTimer)
    const q = query.trim()
    if (!q) {
      // 清空搜索：回到清空前所在的视图（标签 / 笔记本）
      if (selectedTagId) selectTag(selectedTagId, tags.find((x) => x.id === selectedTagId)?.title ?? '')
      else if (selectedFolderId) selectFolder(selectedFolderId)
      return
    }
    searchTimer = setTimeout(async () => {
      try {
        notes = await api.search(q)
        searchMode = true
        listTitle = t('list.searchPrefix', { q })
      } catch (e) {
        error = `${e}`
      }
    }, 200)
  }
</script>

<div class="app">
  <header class="topbar">
    <div class="brand">Jasper</div>
    {#if readOnly}
      <span class="ro-badge" title={t('common.readOnlyTitle')}>{t('common.readOnly')}</span>
    {/if}
    <input
      class="search"
      type="search"
      placeholder={t('topbar.search')}
      bind:value={query}
      oninput={onSearchInput}
    />
    <div class="topbar-actions">
      <LangPicker />
      <ThemePicker />
      <!-- 本地可写应用：真实文件夹数据源（FSA，仅 Chromium）。localFolder 仅 WASM_WRITABLE 构建存在。 -->
      {#if localFolder}
        {#if folderName}
          <span class="source-chip" title={t('local.folderTip', { name: folderName })}>
            <Icon name="folder" size={14} /> {folderName}
          </span>
          <Button variant="ghost" iconOnly icon="close" label={t('local.closeFolder')} onclick={closeFolder} />
        {:else if localFolder.supported()}
          {#if pendingFolder}
            <Button variant="ghost" icon="folder" label={t('local.reconnect', { name: pendingFolder })} onclick={reconnectFolder} />
          {:else}
            <div class="folder-cta">
              <Button variant="ghost" iconOnly icon="folder" label={t('local.openFolder')} title={t('local.openFolderTip')} onclick={openFolder} />
              {#if showFolderHint}
                <div class="folder-hint" role="status" transition:fade={{ duration: 150 }}>
                  <span class="msg">{@html t('local.hint')}</span>
                  <button type="button" class="hint-got" onclick={dismissFolderHint}>{t('local.hintGot')}</button>
                </div>
              {/if}
            </div>
          {/if}
        {:else}
          <span class="source-hint" title={t('local.notSupported')}><Icon name="folder" size={14} /></span>
        {/if}
      {/if}
      {#if !IS_DEMO}
        {#if locked}
          <!-- 受保护未登录：只给「解锁登录」入口，管理功能待登录后出现 -->
          <Button variant="ghost" iconOnly icon="lock" label={t('auth.unlock')} onclick={() => (showAuthDialog = true)} />
        {:else}
          {#if authenticated}
            <Button variant="ghost" iconOnly icon="unlock" label={t('auth.lock')} onclick={doLogout} />
          {/if}
          <Button variant="ghost" iconOnly icon="image" label={t('topbar.resources')} onclick={() => (showResources = true)} />
          {#if pluginsAvailable()}
            <Button variant="ghost" iconOnly icon="plug" label={t('plugins.topbar')} onclick={() => (showPlugins = true)} />
          {/if}
          <!-- 本地 WASM 应用无 server：设置里的数据源/鉴权/AI 段无从谈起，隐藏入口（语言/主题在顶栏独立切换）。 -->
          {#if !IS_WASM}
            <Button variant="ghost" iconOnly icon="settings" label={t('topbar.settings')} onclick={() => (showSettings = true)} />
          {/if}
        {/if}
      {/if}
    </div>
  </header>

  {#if IS_DEMO && showDemoBanner}
    <div class="demo-banner" transition:slide={{ duration: 200 }}>
      <span class="msg">
        {@html t('demo.banner')}
        <span class="dim">{t('demo.bannerDim')}</span>
      </span>
      <Button variant="ghost" iconOnly icon="close" label={t('common.close')} onclick={() => (showDemoBanner = false)} />
    </div>
  {/if}

  {#if error}
    <div class="error" transition:slide={{ duration: 200 }}>{error}</div>
  {/if}

  {#if locked && configured && !passwordlessRead}
    <!-- 受保护、未登录、且未开放无密码阅读（整站私有 → 匿名什么都看不到）→ 登录闸门取代三栏。
         用 passwordlessRead 判定而非 folders.length：后者在重载后的加载间隙短暂为空，会导致闸门闪现
         + 与顶栏「解锁」按钮同名重复。开了无密码阅读则始终展示三栏（只读浏览可见范围内容）。 -->
    <div class="locked-gate">
      <Icon name="lock" size={40} />
      <h2>{t('auth.lockedTitle')}</h2>
      <p>{t('auth.lockedDesc')}</p>
      <Button variant="primary" icon="lock" label={t('auth.unlock')} onclick={() => (showAuthDialog = true)} />
    </div>
  {:else}
    <div class="panes" class:with-dock={dockEntry != null}>
    <aside class="sidebar">
      <div class="pane-title">
        <span>{t('pane.notebooks')}</span>
        {#if !readOnly}
          <Button variant="ghost" iconOnly icon="folder-plus" label={t('pane.newNotebook')} onclick={handleNewFolder} />
        {/if}
      </div>
      {#if !readOnly && draggingFolder()}
        <div
          class="root-drop"
          class:over={rootDropOver}
          role="presentation"
          transition:slide={{ duration: 150 }}
          ondragover={onRootDragOver}
          ondragleave={() => (rootDropOver = false)}
          ondrop={onRootDrop}
        >
          {t('tree.moveToTop')}
        </div>
      {/if}
      <FolderTree
        {folders}
        selectedId={searchMode ? null : selectedFolderId}
        onSelect={(id) => selectFolder(id)}
        onMoveNote={readOnly ? undefined : moveNote}
        onMoveFolder={readOnly ? undefined : moveFolder}
        onRenameFolder={readOnly ? undefined : renameFolder}
        onDeleteFolder={readOnly ? undefined : deleteFolder}
      />
      <TagList {tags} selectedId={searchMode ? null : selectedTagId} onSelect={selectTag} />
      <!-- 插件面板入口（左栏底部，spec §9.4）；只读下命令端点全被拦，一并隐藏 -->
      {#if !readOnly && sidebarContributions().length}
        <div class="plugin-entries">
          {#each sidebarContributions() as entry (entry.pluginId + '/' + entry.contribution.id)}
            <button
              class="plugin-entry"
              class:active={dockEntry?.pluginId === entry.pluginId &&
                dockEntry.contribution.id === entry.contribution.id}
              title={entry.pluginName}
              onclick={() => toggleDock(entry)}
            >
              <Icon name={entry.contribution.icon || 'plug'} size={14} />
              <span class="entry-title">{entry.contribution.title}</span>
            </button>
          {/each}
        </div>
      {/if}
    </aside>

    <section class="notelist">
      <div class="pane-title">
        <span>{listTitle}</span>
        {#if !readOnly}
          <span class="title-actions">
            <Button variant="ghost" iconOnly icon="check-square" label={t('pane.newTodo')} onclick={handleNewTodo} />
            <Button variant="ghost" iconOnly icon="plus" label={t('pane.newNote')} onclick={handleNew} />
          </span>
        {/if}
      </div>
      <NoteList {notes} selectedId={selectedNoteId} onSelect={selectNote} canDrag={!readOnly} />
    </section>

    <main class="reader">
      {#key selectedNoteId}
        <NoteView
          bind:this={noteView}
          {detail}
          onNavigate={navigate}
          onChanged={refreshList}
          onDeleted={onNoteDeleted}
          onTagsChanged={onNoteTagsChanged}
          tagSuggestions={tags.map((t) => t.title)}
          {tagRefreshToken}
          initialEdit={detail != null && !readOnly}
          {readOnly}
          onOpenPdf={openPdf}
        />
      {/key}
    </main>

    {#if dockEntry}
      <aside class="dock">
        {#key dockEntry.pluginId + '/' + dockEntry.contribution.id}
          <PluginSidebar
            entry={dockEntry}
            noteId={selectedNoteId}
            onClose={() => (dockEntry = null)}
            onNotesChanged={refreshList}
          />
        {/key}
      </aside>
    {/if}
    </div>
  {/if}
</div>

<PendingWriteDialog onApplied={onPluginWriteApplied} />

{#if showAuthDialog}
  <AuthDialog onSuccess={onLoginSuccess} onClose={() => (showAuthDialog = false)} />
{/if}

{#if configured === false}
  <Settings mode="setup" onDone={onConfigured} />
{:else if showSettings}
  <Settings
    mode="settings"
    onDone={onConfigured}
    onClose={() => (showSettings = false)}
    onAuthChanged={refreshStatus}
  />
{/if}

{#if showPlugins}
  <PluginPanel {readOnly} onClose={() => (showPlugins = false)} />
{/if}

{#if pdfModal}
  <PdfViewer url={pdfModal.url} name={pdfModal.name} onClose={() => (pdfModal = null)} />
{/if}

{#if showResources}
  <ResourcePanel {readOnly} onClose={() => (showResources = false)} onChanged={onResourcesChanged} />
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 14px;
    height: 44px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-bar);
    flex: 0 0 auto;
  }
  .brand {
    font-weight: 600;
    font-size: 14px;
    color: var(--accent);
  }
  .ro-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--bg-side);
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
    flex: 0 0 auto;
    cursor: default;
  }
  /* 本地应用：真实文件夹数据源指示（chip）/ 不支持提示 */
  .source-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 200px;
    padding: 2px 9px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--bg-side);
    color: var(--text);
    font-size: 12px;
    font-weight: 500;
    flex: 0 0 auto;
    cursor: default;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .source-hint {
    display: inline-flex;
    align-items: center;
    color: var(--text-dim);
    opacity: 0.5;
    cursor: help;
  }
  /* 首次悬浮提示：锚在「打开文件夹」按钮下方 */
  .folder-cta {
    position: relative;
    display: inline-flex;
  }
  .folder-hint {
    position: absolute;
    top: calc(100% + 9px);
    right: 0;
    z-index: 60;
    width: max-content;
    max-width: 250px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 9px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    color: var(--text);
    font-size: 12.5px;
    line-height: 1.55;
    text-align: left;
    white-space: normal;
  }
  .folder-hint::before {
    content: '';
    position: absolute;
    top: -6px;
    right: 12px;
    width: 11px;
    height: 11px;
    background: var(--bg);
    border-left: 1px solid var(--accent);
    border-top: 1px solid var(--accent);
    transform: rotate(45deg);
  }
  .folder-hint .msg :global(b) {
    color: var(--accent);
  }
  .folder-hint .hint-got {
    align-self: flex-end;
    padding: 3px 12px;
    border: none;
    border-radius: 6px;
    background: var(--accent);
    color: #fff;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }
  .folder-hint .hint-got:hover {
    opacity: 0.9;
  }
  .search {
    flex: 1;
    max-width: 420px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 0 10px;
    background: var(--bg);
    color: var(--text);
    font-size: 13px;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }
  .search:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .topbar-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .error {
    background: var(--danger);
    color: var(--on-accent);
    padding: 6px 14px;
    font-size: 13px;
  }
  /* 登录闸门（整站私有、未登录）：占据三栏位置的居中提示 */
  .locked-gate {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 40px 20px;
    color: var(--text-dim);
    text-align: center;
  }
  .locked-gate h2 {
    margin: 0;
    font-size: 18px;
    color: var(--text);
  }
  .locked-gate p {
    margin: 0;
    font-size: 14px;
    max-width: 360px;
  }
  .demo-banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 14px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text);
    background: var(--accent-soft);
    border-bottom: 1px solid var(--border);
    flex: 0 0 auto;
  }
  .demo-banner .dim {
    color: var(--text-dim);
  }
  .demo-banner .msg {
    flex: 1;
  }
  .panes {
    display: grid;
    grid-template-columns: 250px 300px 1fr;
    flex: 1;
    min-height: 0;
  }
  /* 插件面板 dock：在阅读区右侧挤出第四栏（spec §9.4，约 320px） */
  .panes.with-dock {
    grid-template-columns: 250px 300px 1fr 320px;
  }
  .dock {
    border-left: 1px solid var(--border);
    overflow: hidden;
    min-height: 0;
    background: var(--bg-side);
  }
  .sidebar {
    border-right: 1px solid var(--border);
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0 0 12px;
    background: var(--bg-side);
    display: flex;
    flex-direction: column;
  }
  .sidebar :global(> *) {
    flex: 0 0 auto;
  }
  /* 插件入口钉在左栏底部 */
  .plugin-entries {
    margin-top: auto;
    padding: 8px 8px 0;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .plugin-entry {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 8px;
    border: none;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    text-align: left;
    border-radius: 7px;
    cursor: pointer;
  }
  .plugin-entry:hover {
    background: var(--hover);
  }
  .plugin-entry.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .entry-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .notelist {
    border-right: 1px solid var(--border);
    overflow-y: auto;
    background: var(--bg-side);
  }
  /* 左两栏（笔记本树 / 笔记列表）：细「悬浮」滚动条——透明轨道 + 悬停才显现的细拇指，
     用透明 border + background-clip 撑出内边距，让拇指看起来更细、像浮在内容上，不抢视觉。
     （webkit 自定义滚动条是否占位取决于系统「显示滚动条」设置；overlay 模式下零占位。） */
  .sidebar,
  .notelist {
    scrollbar-width: thin;
    scrollbar-color: transparent transparent;
  }
  .sidebar:hover,
  .notelist:hover {
    scrollbar-color: color-mix(in srgb, var(--text) 24%, transparent) transparent;
  }
  .sidebar::-webkit-scrollbar,
  .notelist::-webkit-scrollbar {
    width: 10px;
  }
  .sidebar::-webkit-scrollbar-track,
  .notelist::-webkit-scrollbar-track {
    background: transparent;
  }
  .sidebar::-webkit-scrollbar-thumb,
  .notelist::-webkit-scrollbar-thumb {
    background: transparent;
    border: 3px solid transparent;
    border-radius: 999px;
    background-clip: padding-box;
  }
  .sidebar:hover::-webkit-scrollbar-thumb,
  .notelist:hover::-webkit-scrollbar-thumb {
    background: color-mix(in srgb, var(--text) 24%, transparent);
    background-clip: padding-box;
  }
  .sidebar::-webkit-scrollbar-thumb:hover,
  .notelist::-webkit-scrollbar-thumb:hover {
    background: color-mix(in srgb, var(--text) 42%, transparent);
    background-clip: padding-box;
  }
  .pane-title {
    position: sticky;
    top: 0;
    z-index: 10;
    height: 38px;
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    box-sizing: border-box;
    background: var(--bg-side);
    padding: 0 6px 0 12px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .reader {
    overflow-y: auto;
    background: var(--bg);
  }
  .title-actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .root-drop {
    margin: 6px 8px;
    padding: 9px;
    border: 1px dashed var(--border);
    border-radius: 8px;
    text-align: center;
    font-size: 12px;
    color: var(--text-dim);
    transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
  }
  .root-drop.over {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }
</style>
