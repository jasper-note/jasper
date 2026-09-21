<script lang="ts">
  import { slide } from 'svelte/transition'
  import { cubicOut } from 'svelte/easing'
  import { NOTE_DND_TYPE, FOLDER_DND_TYPE, type FolderNode } from './api'
  import { startFolderDrag, endFolderDrag, draggingFolder } from './dnd.svelte'
  import { isExpanded, toggleExpanded } from './treeExpand.svelte'
  import { t } from './i18n.svelte'
  import Icon from './Icon.svelte'
  import Self from './FolderTree.svelte'

  let {
    folders,
    selectedId,
    onSelect,
    onMoveNote,
    onMoveFolder,
    onRenameFolder,
    onDeleteFolder,
    depth = 0,
  }: {
    folders: FolderNode[]
    selectedId: string | null
    onSelect: (id: string) => void
    onMoveNote?: (noteId: string, folderId: string) => void
    onMoveFolder?: (folderId: string, parentId: string) => void
    onRenameFolder?: (folderId: string, currentTitle: string) => void
    onDeleteFolder?: (folder: FolderNode) => void
    depth?: number
  } = $props()

  // 展开/折叠态记忆到 localStorage（treeExpand.svelte，整棵递归树共享单一 rune 集合，
  // 与 jasper.lastNote / jasper.editor 同姿势）；未记录的默认折叠。

  // 拖拽放置高亮的笔记本 id（本实例内；每个递归实例各自维护）
  let dropId = $state<string | null>(null)

  // 拖拽类型：笔记 / 笔记本 / 不可放置
  function dragKind(e: DragEvent): 'note' | 'folder' | null {
    const types = e.dataTransfer?.types
    if (!types) return null
    if (onMoveNote && types.includes(NOTE_DND_TYPE)) return 'note'
    if (onMoveFolder && types.includes(FOLDER_DND_TYPE)) return 'folder'
    return null
  }
  // 笔记本拖拽时禁止落到自身或其后代（防环），由 dnd 共享态提供 forbidden 集合
  function canDropOn(id: string, kind: 'note' | 'folder'): boolean {
    if (kind === 'folder') {
      const d = draggingFolder()
      if (d && d.forbidden.has(id)) return false
    }
    return true
  }
  function onRowDragOver(e: DragEvent, id: string) {
    const kind = dragKind(e)
    if (!kind || !canDropOn(id, kind)) return
    e.preventDefault() // 允许放置
    e.dataTransfer!.dropEffect = 'move'
    dropId = id
  }
  function onRowDragLeave(e: DragEvent, id: string) {
    // 移到行内子元素（caret/folder 按钮）不算离开，避免高亮闪烁
    const related = e.relatedTarget as Node | null
    if (related && (e.currentTarget as HTMLElement).contains(related)) return
    if (dropId === id) dropId = null
  }
  function onRowDrop(e: DragEvent, id: string) {
    const kind = dragKind(e)
    dropId = null
    if (!kind || !canDropOn(id, kind)) return
    e.preventDefault()
    if (kind === 'note') {
      const noteId = e.dataTransfer!.getData(NOTE_DND_TYPE)
      if (noteId) onMoveNote!(noteId, id)
    } else {
      const folderId = e.dataTransfer!.getData(FOLDER_DND_TYPE)
      if (folderId && folderId !== id) onMoveFolder!(folderId, id)
    }
  }

  // 拖拽笔记本：开始时算好「自身+全部后代」id 集合（用于防环），存入 dnd 共享态
  function collectSubtreeIds(node: FolderNode, into: Set<string>) {
    into.add(node.id)
    for (const c of node.children) collectSubtreeIds(c, into)
  }
  function onFolderDragStart(e: DragEvent, f: FolderNode) {
    if (!onMoveFolder || !e.dataTransfer) return
    const forbidden = new Set<string>()
    collectSubtreeIds(f, forbidden)
    e.dataTransfer.setData(FOLDER_DND_TYPE, f.id)
    e.dataTransfer.effectAllowed = 'move'
    startFolderDrag(f.id, forbidden)
  }
</script>

<ul class="tree">
  {#each folders as f (f.id)}
    <li>
      <div
        class="row"
        class:active={f.id === selectedId}
        class:drop-target={dropId === f.id}
        style="padding-left: {depth * 14 + 6}px"
        role="presentation"
        ondragover={(e) => onRowDragOver(e, f.id)}
        ondragleave={(e) => onRowDragLeave(e, f.id)}
        ondrop={(e) => onRowDrop(e, f.id)}
      >
        {#if f.children.length}
          <button
            class="caret"
            class:open={isExpanded(f.id)}
            onclick={(e) => {
              e.stopPropagation()
              toggleExpanded(f.id)
            }}
            aria-label={t('tree.toggle')}
          >
            <svg viewBox="0 0 10 10" width="9" height="9" aria-hidden="true">
              <path d="M3.5 1.5 L7.5 5 L3.5 8.5 Z" fill="currentColor" />
            </svg>
          </button>
        {:else}
          <span class="caret spacer"></span>
        {/if}

        <button
          class="folder"
          draggable={!!onMoveFolder && f.id !== ''}
          ondragstart={onMoveFolder && f.id !== '' ? (e) => onFolderDragStart(e, f) : undefined}
          ondragend={onMoveFolder && f.id !== '' ? endFolderDrag : undefined}
          onclick={() => onSelect(f.id)}
        >
          <span class="ic"><Icon name={f.children.length ? 'folder' : 'file'} size={14} /></span>
          <span class="name">{f.id === '' ? t('tree.unfiled') : f.title || t('common.unnamed')}</span>
          {#if f.note_count > 0}<span class="count">{f.note_count}</span>{/if}
        </button>

        {#if onRenameFolder && f.id !== ''}
          <button
            class="rowaction"
            title={t('common.rename')}
            aria-label={t('common.rename')}
            onclick={(e) => {
              e.stopPropagation()
              onRenameFolder!(f.id, f.title)
            }}
          >
            <Icon name="edit" size={13} />
          </button>
        {/if}

        {#if onDeleteFolder && f.id !== ''}
          <button
            class="rowaction danger"
            title={t('notebook.delete')}
            aria-label={t('notebook.delete')}
            onclick={(e) => {
              e.stopPropagation()
              onDeleteFolder!(f)
            }}
          >
            <Icon name="trash" size={13} />
          </button>
        {/if}
      </div>

      {#if f.children.length && isExpanded(f.id)}
        <div class="subtree" transition:slide={{ duration: 180, easing: cubicOut }}>
          <Self
            folders={f.children}
            {selectedId}
            {onSelect}
            {onMoveNote}
            {onMoveFolder}
            {onRenameFolder}
            {onDeleteFolder}
            depth={depth + 1}
          />
        </div>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .tree {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    height: 30px;
    margin: 1px 4px;
    border-radius: 7px;
    cursor: pointer;
    transition: background 0.13s ease;
    overflow: hidden;
  }
  .row::before {
    content: '';
    position: absolute;
    left: 0;
    top: 50%;
    width: 3px;
    height: 0;
    background: var(--accent);
    border-radius: 0 2px 2px 0;
    transform: translateY(-50%);
    transition: height 0.18s ease;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.active {
    background: var(--accent-soft);
  }
  .row.active::before {
    height: 62%;
  }
  /* 拖拽笔记悬停其上的放置目标：内描边 + 强调底色（不改变布局） */
  .row.drop-target {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .caret {
    width: 18px;
    height: 18px;
    flex: 0 0 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
  }
  .caret svg {
    transition: transform 0.13s ease;
  }
  .caret.open svg {
    transform: rotate(90deg);
  }
  .caret:hover {
    color: var(--text);
  }
  .caret.spacer {
    cursor: default;
  }
  .folder {
    flex: 1 1 auto;
    display: flex;
    align-items: center;
    gap: 7px;
    background: none;
    border: none;
    color: var(--text);
    text-align: left;
    padding: 0 8px 0 2px;
    height: 100%;
    cursor: pointer;
    font-size: 13px;
    min-width: 0;
  }
  .ic {
    font-size: 12px;
    opacity: 0.85;
    flex: 0 0 auto;
  }
  .name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row.active .name {
    color: var(--accent);
    font-weight: 600;
  }
  .count {
    flex: 0 0 auto;
    margin-left: 6px;
    color: var(--text-dim);
    font-size: 11px;
    background: var(--hover);
    border-radius: 9px;
    padding: 1px 7px;
    min-width: 20px;
    text-align: center;
  }
  .row.active .count {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  /* 行内操作按钮（重命名 / 删除）：常驻占位（不引发悬停时布局跳动），默认隐藏且不可点，
     行悬停或键盘聚焦时显现。 */
  .rowaction {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    margin-right: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: var(--text-dim);
    border-radius: 5px;
    padding: 0;
    cursor: pointer;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s ease, background 0.12s ease, color 0.12s ease;
  }
  .row:hover .rowaction,
  .rowaction:focus-visible {
    opacity: 1;
    pointer-events: auto;
  }
  .rowaction:hover {
    background: var(--hover);
    color: var(--text);
  }
  .rowaction.danger:hover {
    color: var(--danger);
  }
</style>
