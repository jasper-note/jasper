<script lang="ts">
  // 设置面板壳（服务器驱动）：分区侧边栏 + 搜索，右侧用通用渲染器 SettingsSection 渲染当前分区。
  // 分区目录/字段/顺序/可用性全部来自 GET /api/settings/schema，前端不硬编码。
  // 两种用途：mode='setup' 首次向导（聚焦卡片，只渲染数据源分区）；mode='settings' 完整侧边栏壳。
  import { onMount } from 'svelte'
  import { fade, scale } from 'svelte/transition'
  import { cubicOut } from 'svelte/easing'
  import { api } from './api'
  import { t } from './i18n.svelte'
  import Button from './Button.svelte'
  import Icon from './Icon.svelte'
  import SettingsSection from './SettingsSection.svelte'
  import { loadPlugins } from './plugins.svelte'
  import { filterSections, resolveLabel, type SettingsSchema } from './settingsSchema'

  let {
    mode,
    onDone,
    onClose,
    onAuthChanged,
  }: {
    mode: 'setup' | 'settings'
    onDone: () => void
    onClose?: () => void
    // 访问控制设置变更后通知父组件刷新 /api/status（更新只读闸门/登录态）
    onAuthChanged?: () => void
  } = $props()

  // undefined = 加载中；null = 拉取失败（老服务器无此路由等）
  let schema = $state<SettingsSchema | null | undefined>(undefined)
  let activeId = $state('')
  let query = $state('')

  onMount(async () => {
    loadPlugins() // 幂等；保证存储 provider 选项与子表单可用
    schema = await api.settingsSchema()
    if (schema) {
      const first = mode === 'setup' ? schema.sections.find((s) => s.id === 'data-source') : schema.sections[0]
      activeId = first?.id ?? ''
    }
  })

  // 描述符版本号：服务端生成了新值需要回显时（如 MCP 的 API key）+1，参与下面的 {#key}
  // 强制重挂当前分区——SettingsSection 的表单值只在挂载时从描述符算一次，光换 prop 不会重算。
  let schemaRev = $state(0)
  async function reloadSchema() {
    const fresh = await api.settingsSchema()
    if (fresh) {
      schema = fresh
      schemaRev++
    }
  }

  const sections = $derived(schema?.sections ?? [])
  const setupSection = $derived(sections.find((s) => s.id === 'data-source'))
  const activeSection = $derived(sections.find((s) => s.id === activeId))
  const filtered = $derived(filterSections(sections, query))

  function select(id: string) {
    activeId = id
    query = ''
  }
</script>

{#if mode === 'setup'}
  <div class="overlay setup" transition:fade={{ duration: 150 }}>
    <div class="card setup-card" transition:scale={{ duration: 190, start: 0.96, opacity: 0, easing: cubicOut }}>
      <header><h2>{t('settings.welcomeTitle')}</h2></header>
      <p class="hint">{t('settings.firstHint')}</p>
      {#if schema === undefined}
        <p class="loading">{t('common.loading')}</p>
      {:else if setupSection}
        {#key setupSection.id}
          <SettingsSection section={setupSection} {onDone} />
        {/key}
      {:else}
        <div class="error"><Icon name="alert" size={14} /> {t('settings.connectFailed')}</div>
      {/if}
    </div>
  </div>
{:else}
  <div class="overlay" transition:fade={{ duration: 150 }}>
    <div class="card shell" transition:scale={{ duration: 190, start: 0.96, opacity: 0, easing: cubicOut }}>
      <aside class="nav">
        <div class="nav-head">
          <h2>{t('settings.title')}</h2>
          <Button variant="ghost" iconOnly icon="close" label={t('common.close')} onclick={() => onClose?.()} />
        </div>
        <input class="search" type="search" placeholder={t('settings.search.placeholder')} bind:value={query} />
        <nav class="section-list">
          {#if filtered.length === 0}
            {#if query}<p class="no-results">{t('settings.search.noResults')}</p>{/if}
          {:else}
            {#each filtered as s (s.id)}
              <button class="nav-item" class:active={s.id === activeId} onclick={() => select(s.id)}>
                <Icon name={s.icon} size={15} />
                <span>{resolveLabel(s.title_key)}</span>
              </button>
            {/each}
          {/if}
        </nav>
      </aside>
      <div class="content">
        {#if schema === undefined}
          <p class="loading">{t('common.loading')}</p>
        {:else if schema === null}
          <div class="error"><Icon name="alert" size={14} /> {t('settings.connectFailed')}</div>
        {:else if activeSection}
          {#key `${activeSection.id}:${schemaRev}`}
            <SettingsSection
              section={activeSection}
              {onDone}
              {onAuthChanged}
              onReloadSection={reloadSchema}
            />
          {/key}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--overlay);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .overlay.setup {
    background: var(--bg);
  }
  .card {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow-modal);
  }
  /* 首次向导：聚焦单卡片，只放数据源分区 */
  .setup-card {
    width: 460px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 32px);
    overflow-y: auto;
    padding: 22px 24px 24px;
  }
  .setup-card header {
    margin-bottom: 4px;
  }
  /* 设置：两栏壳（侧边栏 + 内容） */
  .shell {
    display: flex;
    width: 760px;
    max-width: calc(100vw - 32px);
    height: 560px;
    max-height: calc(100vh - 32px);
    overflow: hidden;
  }
  .nav {
    flex: 0 0 208px;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    background: var(--bg-side);
    padding: 14px 12px;
    gap: 10px;
  }
  .nav-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 17px;
  }
  .search {
    width: 100%;
    box-sizing: border-box;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .search:focus {
    outline: none;
    border-color: var(--accent);
  }
  .section-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
  }
  .nav-item {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    text-align: left;
    padding: 8px 10px;
    border: none;
    border-radius: 7px;
    background: none;
    color: var(--text);
    cursor: pointer;
    font-size: 13px;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .nav-item:hover {
    background: var(--hover, rgba(127, 127, 127, 0.1));
  }
  .nav-item.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }
  .no-results {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 10px;
  }
  .content {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 22px 26px;
  }
  .hint {
    color: var(--text-dim);
    font-size: 13px;
    margin: 6px 0 4px;
  }
  .loading {
    color: var(--text-dim);
    font-size: 13px;
  }
  .error {
    margin-top: 14px;
    padding: 8px 10px;
    background: var(--danger-soft);
    color: var(--danger);
    border-radius: 7px;
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
