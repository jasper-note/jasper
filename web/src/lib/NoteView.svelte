<script lang="ts">
  import { onDestroy } from 'svelte'
  import { fade } from 'svelte/transition'
  import type { NoteDetail } from './api'
  import { api, taskProgress } from './api'
  import { t } from './i18n.svelte'
  import { renderNote } from './render'
  import Button from './Button.svelte'
  import CodeMirrorEditor from './CodeMirrorEditor.svelte'
  import EditorToolbar from './EditorToolbar.svelte'
  import NoteTags from './NoteTags.svelte'
  import Icon from './Icon.svelte'
  import type { EditorHandle } from './editor/types'
  import { editorCommands } from './plugins.svelte'
  import { enqueuePendingWrites } from './pendingWrites.svelte'
  import { getEngine, setEngine, getContentWidth, setContentWidth, type ContentWidth } from './editorPrefs'
  import { renderPdfInto, type PdfHandle } from './pdfRender'
  import { Autosave } from './autosave'

  let {
    detail,
    onNavigate,
    onChanged,
    onDeleted,
    onTagsChanged,
    tagSuggestions = [],
    tagRefreshToken = 0,
    initialEdit = false,
    readOnly = false,
    onOpenPdf,
  }: {
    detail: NoteDetail | null
    onNavigate: (id: string) => void
    onChanged: () => void
    onDeleted: () => void
    onTagsChanged?: () => void
    tagSuggestions?: string[]
    tagRefreshToken?: number
    initialEdit?: boolean
    readOnly?: boolean
    onOpenPdf?: (id: string, name: string) => void
  } = $props()

  // 本组件按笔记 id 在父级以 {#key} 重挂载，故这里用初始值即可，无需响应 detail 变化。
  let editMode = $state(initialEdit)
  let title = $state(detail?.title ?? '')
  let body = $state(detail?.body ?? '')
  // 编辑器句柄（就绪后由 CodeMirrorEditor 回传），供工具栏/插件命令操作当前源码。
  // 统一到一个 CM6 实例后，源码与 Live Preview 共用同一句柄。
  let editorHandle = $state<EditorHandle | null>(null)

  // 编辑视图：Live Preview 实时预览 / 源码，均为同一个 CodeMirror 实例，记忆在 localStorage。
  // HTML 笔记（markup_language=2）不解析 markdown，强制源码。
  let isMarkdown = $derived(detail?.markup_language !== 2)
  // 默认视图偏好由设置面板「编辑器」分区与本组件共享（editorPrefs，jasper.editor 键）。
  let editorEngine = $state<'live' | 'source'>(getEngine())
  let engine = $derived<'live' | 'source'>(isMarkdown ? editorEngine : 'source')
  function toggleEngine() {
    editorEngine = editorEngine === 'live' ? 'source' : 'live'
    setEngine(editorEngine)
  }

  // 内容宽度：铺满 / 居中限宽（编辑与阅读共用），记忆在 localStorage
  let contentWidth = $state<ContentWidth>(getContentWidth())
  function toggleWidth() {
    contentWidth = contentWidth === 'full' ? 'centered' : 'full'
    setContentWidth(contentWidth)
  }

  // 自动保存协调（脏标记/在途/快照复核/外部回显门控）抽到纯逻辑 Autosave，见 autosave.ts
  const autosave = new Autosave()
  // 正在把外部变更写进编辑器缓冲：其间 setValue 触发的 onChange 是程序化写入，不应排自动保存
  let applyingExternal = false
  let saveState = $state<'idle' | 'saving' | 'saved' | 'error'>('idle')
  let timer: ReturnType<typeof setTimeout> | undefined

  // 阅读视图按本地（可能已编辑）的标题/正文渲染，保证切回阅读即时反映改动
  let html = $derived(
    detail ? renderNote({ ...detail, title, body }) : ''
  )
  // 任务清单进度（随正文实时变化）
  let tasks = $derived(taskProgress(body))

  function scheduleSave() {
    if (!detail) return
    autosave.markDirty()
    saveState = 'saving'
    clearTimeout(timer)
    timer = setTimeout(save, 800)
  }

  async function save() {
    if (!detail || !autosave.canBeginSave()) return
    clearTimeout(timer)
    const id = detail.id
    // 快照本次要保存的内容：await 期间用户可能继续打字，届时不能把缓冲当作已保存——否则
    // dirty 被误清后，自己写入的 SSE 回声会经 applyExternal 把更晚的输入连同光标一起重置回
    // 服务端旧值（正是「编辑到一半被重置」的根因）。见 autosave.ts。
    const saved = { title, body }
    autosave.beginSave()
    try {
      await api.updateNote(id, saved)
      onChanged()
      if (autosave.finishSaveOk({ title, body }, saved)) {
        saveState = 'saved'
      } else {
        // 保存期间又有新输入：仍是脏的，继续调度下一次保存，绝不清 dirty、绝不回退缓冲。
        scheduleSave()
      }
    } catch {
      autosave.finishSaveErr()
      saveState = 'error'
    }
  }

  // Cmd/Ctrl+S：App 的 window 监听拦下浏览器的「保存网页」后调这里——不等防抖，立即冲刷未保存改动。
  // 无改动时给一次「已保存」反馈，否则按键毫无回应（保存态在编辑态工具栏里显示）。
  export function saveNow() {
    if (!detail || readOnly) return
    if (autosave.dirty) {
      if (!autosave.saving) void save() // 在途保存返回时会自行续排下一次，无需并发
    } else {
      saveState = 'saved'
    }
  }

  function onBodyChange(v: string) {
    body = v
    if (applyingExternal) return // 程序化写入（外部同步）不排自动保存
    scheduleSave()
  }

  // ---------- 附件上传（工具栏「附件」按钮 / 粘贴 / 拖拽） ----------
  // 上传后把 :/id 引用插入编辑缓冲；按钮在工具栏，粘贴/拖拽经 CodeMirrorEditor 的 onFiles 回传。
  let fileInput = $state<HTMLInputElement | null>(null)
  let uploading = $state(0)
  let uploadErr = $state('')

  function attachName(file: File): string {
    if (file.name) return file.name
    const ext = (file.type.split('/')[1] || 'bin').replace('+xml', '')
    return `pasted-${Date.now()}.${ext}`
  }

  async function uploadFiles(files: File[]) {
    if (!files.length || !editorHandle || readOnly) return
    uploadErr = ''
    for (const file of files) {
      uploading++
      try {
        const r = await api.uploadResource(file, attachName(file))
        editorHandle.insert(r.markdown + '\n')
      } catch (e) {
        uploadErr = (e as Error).message || t('editor.uploadFailed')
      } finally {
        uploading--
      }
    }
  }

  function pickFile() {
    fileInput?.click()
  }
  function onPick() {
    if (fileInput?.files) uploadFiles(Array.from(fileInput.files))
    if (fileInput) fileInput.value = ''
  }

  // SSE 外部变更（免确认直写/别的客户端/curl）的保守回显（design doc §5.3）：
  // 仅当 (a) 本地无未保存输入、(b) 服务端内容确与缓冲不同 时才替换——绝不打断正在输入的用户。
  // 统一到 CM6 后源码/Live Preview 同一实例，可无损 setValue（不再有富文本跳过分支）。
  // 返回是否已应用，父级据此同步自己的 detail 快照。
  export function applyExternal(fresh: NoteDetail): boolean {
    if (!detail || fresh.id !== detail.id) return false
    if (!autosave.canApplyExternal({ title, body }, { title: fresh.title, body: fresh.body })) return false
    applyingExternal = true
    title = fresh.title
    body = fresh.body
    editorHandle?.setValue(body) // 编辑器视图同步（阅读视图经 html derived 自动更新）
    applyingExternal = false
    return true
  }

  // 插件 backend 命令（note-toolbar）：把当前正文交给命令，返回的 body 替换编辑缓冲。
  // 统一到 CM6 后源码/Live Preview 均可安全替换正文，两模式都暴露。
  let runningCmd = $state<string | null>(null)
  let cmdError = $state('')
  async function runPluginCommand(pluginId: string, commandId: string) {
    if (!detail || runningCmd) return
    runningCmd = commandId
    cmdError = ''
    try {
      const r = await api.runPluginCommand(pluginId, commandId, {
        note_id: detail.id,
        title,
        body,
      })
      enqueuePendingWrites(r.pending_writes) // 写提案交全局确认队列（spec 0.3 §9.5）
      const result = r.result
      if (typeof result.body === 'string' && result.body !== body) {
        body = result.body
        editorHandle?.setValue(body) // 同步编辑器视图
        scheduleSave() // 走正常自动保存链路
      }
    } catch (e) {
      cmdError = e instanceof Error ? e.message : `${e}`
    } finally {
      runningCmd = null
    }
  }

  async function remove() {
    if (!detail) return
    if (!confirm(t('note.confirmDelete', { title: title || t('common.untitled') }))) return
    try {
      await api.deleteNote(detail.id)
      onDeleted()
    } catch {
      saveState = 'error'
    }
  }

  // 切换笔记/卸载时，若有未保存改动则立即冲刷
  onDestroy(() => {
    if (autosave.dirty && detail) {
      clearTimeout(timer)
      const id = detail.id
      api.updateNote(id, { title, body }).then(onChanged).catch(() => {})
    }
  })

  function fmtDateTime(ms: number): string {
    if (!ms) return ''
    return new Date(ms).toLocaleString()
  }

  function onContentClick(e: MouseEvent) {
    const el = (e.target as HTMLElement).closest('[data-internal-id]') as HTMLElement | null
    if (!el) return
    e.preventDefault()
    const id = el.getAttribute('data-internal-id')
    if (id) onNavigate(id)
  }

  // 内联 PDF「全屏」按钮（编辑器 widget）→ 冒泡到 window 的 jasper-open-pdf → 打开全屏模态
  $effect(() => {
    const h = (e: Event) => {
      const d = (e as CustomEvent).detail
      if (d?.id) onOpenPdf?.(d.id, d.name || 'PDF')
    }
    window.addEventListener('jasper-open-pdf', h)
    return () => window.removeEventListener('jasper-open-pdf', h)
  })

  // 阅读视图里的 PDF：往 render.ts 产出的 .pdf-embed 占位盒内挂 pdfRender（内联嵌入）。
  // html 或阅读/编辑态变化时重挂；cleanup 销毁旧实例（含 pdf.js 资源）。
  let contentEl = $state<HTMLDivElement>()
  $effect(() => {
    void html
    void editMode
    if (editMode || !contentEl) return
    const handles: PdfHandle[] = []
    contentEl.querySelectorAll<HTMLElement>('.pdf-embed').forEach((box) => {
      const id = box.getAttribute('data-resource-id')
      if (!id) return
      const nm = box.getAttribute('data-filename') || 'PDF'
      handles.push(renderPdfInto(box, { url: api.resourceUrl(id), name: nm, id, onExpand: () => onOpenPdf?.(id, nm) }))
    })
    return () => handles.forEach((h) => h.destroy())
  })
</script>

{#if detail}
  <article
    class="note-view"
    class:width-full={contentWidth === 'full'}
    class:width-centered={contentWidth === 'centered'}
    in:fade={{ duration: 160 }}
  >
    <div class="toolbar">
      <div class="left">
        {#if editMode && !readOnly}
          <EditorToolbar mode={engine} handle={editorHandle} />
          <span class="sep"></span>
          <!-- 附件：上传后插入 :/id 引用（也支持直接粘贴/拖拽图片） -->
          <Button
            variant="ghost"
            iconOnly
            icon={uploading > 0 ? 'clean' : 'attach'}
            label={t('editor.attach')}
            title={t('editor.attach') + ' · ' + t('editor.hint')}
            onclick={pickFile}
            disabled={!editorHandle || uploading > 0}
          />
          <input type="file" multiple bind:this={fileInput} onchange={onPick} hidden />
          {#if uploadErr}
            <span class="cmd-error" title={uploadErr}><Icon name="alert" size={12} /> {t('editor.uploadFailed')}</span>
          {/if}
          <!-- 插件贡献的编辑器命令（源码/实时预览均可）：一键优化等 -->
          {#if editorCommands().length}
            <span class="sep"></span>
            {#each editorCommands() as cmd (cmd.pluginId + ':' + cmd.commandId)}
              <Button
                variant="ghost"
                iconOnly
                icon={runningCmd === cmd.commandId ? 'clean' : cmd.icon}
                label={cmd.title}
                onclick={() => runPluginCommand(cmd.pluginId, cmd.commandId)}
                disabled={!editorHandle || runningCmd !== null}
              />
            {/each}
          {/if}
        {/if}
        {#if editMode}
          <span class="save-state {saveState}">
            {#if cmdError}
              <span class="cmd-error" title={cmdError}><Icon name="alert" size={12} /> {t('plugins.cmdFailed')}</span>
            {:else}
              {saveState === 'saving' ? t('note.saving') : saveState === 'saved' ? t('note.saved') : saveState === 'error' ? t('note.saveFailed') : ''}
            {/if}
          </span>
        {/if}
      </div>
      <div class="right">
        <Button
          variant="default"
          iconOnly
          icon={contentWidth === 'full' ? 'align-justify' : 'align-center'}
          label={t('note.contentWidth')}
          onclick={toggleWidth}
        />
        {#if !readOnly}
          {#if editMode && isMarkdown}
            <Button
              variant="default"
              icon={engine === 'live' ? 'code' : 'rich'}
              label={engine === 'live' ? t('note.toSource') : t('note.toRich')}
              onclick={toggleEngine}
            />
          {/if}
          <Button
            variant="default"
            icon={editMode ? 'eye' : 'edit'}
            label={editMode ? t('note.read') : t('note.edit')}
            onclick={() => (editMode = !editMode)}
          />
          <Button variant="danger" icon="trash" label={t('note.delete')} onclick={remove} />
        {/if}
      </div>
    </div>

    <NoteTags
      noteId={detail.id}
      {readOnly}
      suggestions={tagSuggestions}
      onChanged={onTagsChanged}
      refreshToken={tagRefreshToken}
    />

    {#if editMode}
      <input
        class="title-input"
        bind:value={title}
        oninput={scheduleSave}
        placeholder={t('note.titlePlaceholder')}
      />
      <div class="editor-wrap" data-ai-selectable>
        <CodeMirrorEditor
          value={body}
          onChange={onBodyChange}
          onReady={(h) => (editorHandle = h)}
          onFiles={uploadFiles}
          mode={engine}
          {readOnly}
        />
      </div>
    {:else}
      <h1 class="note-title">{title || t('common.untitled')}</h1>
      <div class="meta">
        {t('note.updatedAt', { time: fmtDateTime(detail.updated_time) })}
        {#if detail.source_url}
          · <a href={detail.source_url} target="_blank" rel="noopener noreferrer">{t('note.source')}</a>
        {/if}
        {#if detail.markup_language === 2}<span class="badge">HTML</span>{/if}
        {#if tasks[1] > 0}
          <span class="task-meta" class:done={tasks[0] === tasks[1]}>
            <span class="bar"><span class="fill" style="width:{Math.round((tasks[0] / tasks[1]) * 100)}%"></span></span>
            {t('list.tasks', { done: tasks[0], total: tasks[1] })}
          </span>
        {/if}
      </div>
      <!-- 滚动容器占满整栏宽 → 滚动条贴窗口右缘；内层 .content 才是阅读宽度 -->
      <div class="content-scroll">
        <!-- 内容已由 DOMPurify 净化 -->
        <div class="content" data-ai-selectable bind:this={contentEl} onclick={onContentClick}>{@html html}</div>
      </div>
    {/if}
  </article>
{:else}
  <div class="placeholder">{t('note.placeholder')}</div>
{/if}

<style>
  .note-view {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    /* 高度/下边框与左两栏 .pane-title(38px) 对齐，三栏顶栏齐平 */
    height: 38px;
    box-sizing: border-box;
    padding: 0 8px 0 12px;
    border-bottom: 1px solid var(--border);
    flex: 0 0 auto;
  }
  .save-state {
    font-size: 12px;
    color: var(--text-dim);
    transition: color 0.2s ease;
  }
  .save-state.saved {
    color: var(--success);
  }
  .save-state.error {
    color: var(--danger);
  }
  .cmd-error {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--danger);
  }
  .sep {
    flex: 0 0 auto;
    width: 1px;
    height: 18px;
    background: var(--border);
    margin: 0 3px;
  }
  .left {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1 1 auto;
    overflow: hidden;
  }
  .right {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 0 0 auto;
  }
  .title-input {
    border: none;
    outline: none;
    background: none;
    color: var(--text);
    font-size: 22px;
    font-weight: 700;
    padding: 18px 36px 8px;
    flex: 0 0 auto;
  }
  .editor-wrap {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    /* 不设左右 padding：让 CodeMirror 铺满、滚动条贴右缘（水平留白移到 .cm-content 自身，
       见 theme.ts）。否则滚动条会离右边缘缩进一段。 */
    padding: 0;
  }
  .note-title {
    font-size: 24px;
    margin: 0;
    padding: 24px 36px 6px;
  }
  .meta {
    font-size: 12px;
    color: var(--text-dim);
    padding: 0 36px 14px;
    border-bottom: 1px solid var(--border);
  }
  .badge {
    background: var(--border);
    border-radius: 4px;
    padding: 1px 6px;
    margin-left: 6px;
  }
  .task-meta {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 8px;
    font-variant-numeric: tabular-nums;
  }
  .task-meta .bar {
    width: 60px;
    height: 5px;
    border-radius: 3px;
    background: var(--hover);
    overflow: hidden;
  }
  .task-meta .fill {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
    transition: width 0.2s ease;
  }
  .task-meta.done {
    color: var(--success);
  }
  .task-meta.done .fill {
    background: var(--success);
  }
  /* 全宽滚动容器：滚动条落在整栏（即窗口）右缘 */
  .content-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  /* 阅读内容盒（滚动条不受其 max-width 约束，落在整栏右缘） */
  .content {
    padding: 20px 36px 80px;
  }

  /* 内容宽度：'full' 铺满整栏 / 'centered' 限宽居中（编辑 + 阅读共用，工具栏切换、记忆见 editorPrefs）。
     标题/元信息/正文/CodeMirror 内容统一按 --content-max 限宽并水平居中。 */
  .note-view.width-centered {
    --content-max: 820px;
  }
  .note-view.width-full {
    --content-max: none;
  }
  .title-input,
  .note-title,
  .meta,
  .content,
  .note-view :global(.cm-content) {
    box-sizing: border-box;
    width: 100%;
    max-width: var(--content-max);
    margin-left: auto;
    margin-right: auto;
  }
  .placeholder {
    display: flex;
    height: 100%;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
  }
</style>
