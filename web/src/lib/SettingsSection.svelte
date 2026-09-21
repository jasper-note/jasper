<script lang="ts">
  // 通用设置分区渲染器：由服务器下发的 SettingsSection 描述符驱动，无需为每个分区写专用组件。
  // 按 field.type 渲染（text/secret/multiline/number/bool/enum/notebook-multiselect/theme/language/
  // provider-config），show_if 条件显隐，options_source 动态选项，actions 按 config-result/status
  // 约定提交 + on_success（reload/relogin/saved）。纯逻辑在 settingsSchema.ts（可单测）。
  import { onMount } from 'svelte'
  import { api, getAuthToken, type FolderNode } from './api'
  import { t, getLocale, setLocale, availableLocales, localeName } from './i18n.svelte'
  import Icon from './Icon.svelte'
  import Button from './Button.svelte'
  import SchemaForm from './SchemaForm.svelte'
  import ThemePicker from './ThemePicker.svelte'
  import { defaultValues, validate, type FieldError } from './schema'
  import { storageProviders, pluginsLoaded, type StorageProvider } from './plugins.svelte'
  import {
    evalShowIf,
    resolveLabel,
    fillPlaceholders,
    buildRequestBody,
    interpretResult,
    readClientValue,
    writeClientValue,
    type SettingsSection,
    type SettingsField,
    type SettingsAction,
  } from './settingsSchema'

  let {
    section,
    onDone,
    onAuthChanged,
  }: {
    section: SettingsSection
    // 数据源 connect 成功（on_success=reload）：关闭设置并整库刷新
    onDone: () => void
    // 访问控制保存（on_success=relogin）：通知父组件刷新 /api/status
    onAuthChanged?: () => void
  } = $props()

  const providerKeyOf = (p: StorageProvider) => `plugin:${p.pluginId}:${p.contribution.id}`
  const providerByKey = (key: string) => storageProviders().find((p) => providerKeyOf(p) === key)

  function fieldDefault(f: SettingsField): unknown {
    if (f.default !== undefined) return f.default
    if (f.type === 'bool') return false
    if (f.type === 'enum') return f.options?.[0]?.value ?? ''
    if (f.type === 'notebook-multiselect') return []
    if (f.type === 'provider-config') return {}
    return ''
  }

  // 初始表单值：client 作用域读 localStorage；server 作用域取描述符 values；再并入 values 里的
  // 只读标记（如 password_set）供 show_if/secret-set 用。
  function initValues(): Record<string, unknown> {
    const out: Record<string, unknown> = {}
    for (const f of section.fields) {
      out[f.key] =
        section.scope === 'client' ? readClientValue(f) : section.values?.[f.key] ?? fieldDefault(f)
    }
    if (section.values) {
      for (const [k, v] of Object.entries(section.values)) if (!(k in out)) out[k] = v
    }
    return out
  }

  let values = $state<Record<string, unknown>>(initValues())
  // provider-config 子表单值单独持有（SchemaForm 需可绑定的 Record；bind 不能带类型断言）。
  let pluginConfig = $state<Record<string, unknown>>(
    (section.values?.plugin_config as Record<string, unknown>) ?? {},
  )
  let formErrors = $state<Partial<Record<string, FieldError>>>({})
  let saving = $state(false)
  let saved = $state(false)
  let error = $state('')

  // notebook-multiselect 候选：展平的笔记本树（options_source=folders）。
  let folders = $state<{ id: string; title: string; depth: number }[]>([])
  const needsFolders = section.fields.some((f) => f.options_source === 'folders')

  onMount(async () => {
    if (needsFolders) {
      try {
        folders = flatten(await api.folders())
      } catch {
        folders = []
      }
    }
  })

  function flatten(nodes: FolderNode[], depth = 0): { id: string; title: string; depth: number }[] {
    const out: { id: string; title: string; depth: number }[] = []
    for (const n of nodes) {
      if (n.id) out.push({ id: n.id, title: n.title, depth })
      out.push(...flatten(n.children, depth + 1))
    }
    return out
  }

  // enum 选项 = 描述符声明 + 动态来源（存储 provider）。
  function enumOptions(f: SettingsField): { value: string; label: string }[] {
    const base = (f.options ?? []).map((o) => ({ value: o.value, label: resolveLabel(o.label_key) }))
    if (f.options_source === 'storage_providers') {
      for (const p of storageProviders()) base.push({ value: providerKeyOf(p), label: p.contribution.name })
    }
    return base
  }

  const currentProvider = $derived(providerByKey(String(values.source_type ?? '')))
  // 仅数据源分区有 source_type 字段；其它分区（AI/访问控制/…）无此字段，不得误判为插件源，
  // 否则 prefillMissing 会恒真、禁用它们的 primary 动作按钮。
  const isPluginSource = $derived.by(() => {
    if (section.id !== 'data-source') return false
    const st = String(values.source_type ?? '')
    return st !== 'local' && st !== 'webdav'
  })
  // 回显的 provider 已消失（插件被停用/卸载）→ 警告并禁提交
  const prefillMissing = $derived(isPluginSource && pluginsLoaded() && !currentProvider)

  function setSourceType(value: string) {
    values.source_type = value
    formErrors = {}
    if (value !== 'local' && value !== 'webdav') {
      const prov = providerByKey(value)
      if (prov) {
        // 回到初始 provider → 用服务器回显的 plugin_config 预填；否则用 schema 默认值
        pluginConfig =
          value === section.values?.source_type
            ? { ...((section.values?.plugin_config as Record<string, unknown>) ?? {}) }
            : defaultValues(prov.contribution.config_schema)
      }
    }
  }

  function onEnumClick(f: SettingsField, value: string) {
    if (f.key === 'source_type') {
      setSourceType(value)
      return
    }
    values[f.key] = value
    if (section.scope === 'client') writeClientValue(f, value)
  }

  function setBool(f: SettingsField, checked: boolean) {
    values[f.key] = checked
    if (section.scope === 'client') writeClientValue(f, checked)
  }

  function setText(f: SettingsField, v: string) {
    values[f.key] = v
    if (section.scope === 'client') writeClientValue(f, v)
  }

  function toggleFolder(fkey: string, id: string) {
    const cur = (values[fkey] as string[]) ?? []
    values[fkey] = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id]
  }

  function flashSaved() {
    saved = true
    setTimeout(() => (saved = false), 2000)
  }

  async function runAction(action: SettingsAction) {
    error = ''
    formErrors = {}
    let submitValues = values
    // 数据源插件 provider：先按其 config_schema 校验/清洗子表单
    if (section.id === 'data-source' && isPluginSource && action.submit !== false) {
      const prov = currentProvider
      if (!prov) {
        error = t('settings.providerMissing')
        return
      }
      const v = validate(prov.contribution.config_schema, pluginConfig)
      formErrors = v.errors
      if (!v.ok) return
      submitValues = { ...values, plugin_config: v.cleaned }
    }
    saving = true
    try {
      const body = buildRequestBody(section.id, action, submitValues, section.fields)
      const res = await api.sendSettingsAction(action.request.method, action.request.url, body)
      const result = interpretResult(action.request.convention, res.ok, res.body)
      if (!result.ok) {
        error = result.error || t('settings.connectFailed')
        return
      }
      await handleSuccess(action, res.body)
    } catch (e) {
      error = e instanceof Error ? e.message : `${e}`
    } finally {
      saving = false
    }
  }

  async function handleSuccess(action: SettingsAction, respBody: unknown) {
    switch (action.on_success) {
      case 'reload':
        onDone()
        break
      case 'relogin': {
        // 设/改密码会吊销所有会话（含本人）→ 用刚设的密码自动重登，保持管理员在线。
        const pw = String(values.password ?? '').trim()
        if (pw) {
          try {
            await api.login(pw)
          } catch {
            /* 忽略：父组件刷新状态后会显示登录闸门 */
          }
        }
        values.password = ''
        // 同步 password_set（保存返回 AuthSettingsResp）→ 清除按钮/占位符即时更新
        const rb = respBody as { password_set?: boolean } | null
        if (rb && typeof rb.password_set === 'boolean') values.password_set = rb.password_set
        onAuthChanged?.()
        flashSaved()
        break
      }
      case 'saved':
        flashSaved()
        break
      default:
        break
    }
  }

  function secretPlaceholder(f: SettingsField): string {
    if (f.writeonly && f.set_flag && values[f.set_flag]) return t('form.secretSetPh')
    return resolveLabel(f.placeholder_key)
  }

  // 展示型字段（copy/note）的文本：服务端下发的是模板，这里补上它不可能知道的运行时信息
  // （当前访问地址、浏览器里的会话 token）。
  function displayText(f: SettingsField): string {
    return fillPlaceholders(String(values[f.key] ?? ''), window.location.origin, getAuthToken())
  }

  // 复制成功的字段 key（图标短暂变对勾）。
  let copiedKey = $state<string | null>(null)
  async function copyField(f: SettingsField) {
    try {
      await navigator.clipboard.writeText(displayText(f))
      copiedKey = f.key
      setTimeout(() => (copiedKey = null), 1500)
    } catch {
      /* 剪贴板不可用（非安全上下文/无权限）：输入框是可选中的，用户可手动复制 */
    }
  }
</script>

<div class="section">
  <h3>{resolveLabel(section.title_key)}</h3>
  {#if section.desc_key}<p class="hint">{resolveLabel(section.desc_key)}</p>{/if}

  {#each section.fields as f (f.key)}
    {#if evalShowIf(f.show_if, values)}
      {#if f.type === 'bool'}
        <label class="toggle">
          <input type="checkbox" checked={Boolean(values[f.key])} onchange={(e) => setBool(f, e.currentTarget.checked)} />
          <span class="toggle-body">
            <span class="toggle-title">{resolveLabel(f.label_key)}</span>
            {#if f.desc_key}<span class="toggle-desc">{resolveLabel(f.desc_key)}</span>{/if}
          </span>
        </label>
      {:else if f.type === 'enum'}
        <div class="field">
          {#if f.label_key}<span class="field-label">{resolveLabel(f.label_key)}</span>{/if}
          <div class="seg seg-wrap">
            {#each enumOptions(f) as opt (opt.value)}
              <button class:on={values[f.key] === opt.value} onclick={() => onEnumClick(f, opt.value)}>
                {opt.label}
              </button>
            {/each}
          </div>
          {#if f.desc_key}<p class="tip">{resolveLabel(f.desc_key)}</p>{/if}
        </div>
      {:else if f.type === 'notebook-multiselect'}
        <div class="field">
          {#if f.label_key}<span class="field-label">{resolveLabel(f.label_key)}</span>{/if}
          {#if folders.length === 0}
            <p class="tip">{resolveLabel(f.empty_key)}</p>
          {:else}
            <div class="folder-list">
              {#each folders as fol (fol.id)}
                <label class="folder-check" style="padding-left: {fol.depth * 14}px">
                  <input
                    type="checkbox"
                    checked={((values[f.key] as string[]) ?? []).includes(fol.id)}
                    onchange={() => toggleFolder(f.key, fol.id)}
                  />
                  <span>{fol.title || t('common.untitled')}</span>
                </label>
              {/each}
            </div>
          {/if}
        </div>
      {:else if f.type === 'provider-config'}
        {#if prefillMissing}
          <div class="error"><Icon name="alert" size={14} /> {t('settings.providerMissing')}</div>
        {:else if currentProvider}
          <div class="plugin-form">
            {#key currentProvider.contribution.id}
              <SchemaForm schema={currentProvider.contribution.config_schema} bind:values={pluginConfig} errors={formErrors} />
            {/key}
          </div>
        {/if}
      {:else if f.type === 'theme'}
        <div class="field row">
          <span class="field-label">{resolveLabel(f.label_key)}</span>
          <ThemePicker />
        </div>
      {:else if f.type === 'language'}
        <div class="field">
          <span class="field-label">{resolveLabel(f.label_key)}</span>
          <div class="seg">
            {#each availableLocales() as loc (loc.code)}
              <button class:on={getLocale() === loc.code} onclick={() => setLocale(loc.code)}>
                {localeName(loc.code)}
              </button>
            {/each}
          </div>
        </div>
      {:else if f.type === 'copy'}
        <div class="field">
          {#if f.label_key}<span class="field-label">{resolveLabel(f.label_key)}</span>{/if}
          <div class="copy-row">
            <input class="copy-input" readonly value={displayText(f)} onfocus={(e) => e.currentTarget.select()} />
            <Button
              variant="ghost"
              iconOnly
              icon={copiedKey === f.key ? 'check' : 'braces'}
              label={t('common.copy')}
              onclick={() => copyField(f)}
            />
          </div>
          {#if f.desc_key}<p class="tip">{resolveLabel(f.desc_key)}</p>{/if}
        </div>
      {:else if f.type === 'note'}
        <div class="field">
          {#if f.label_key}<span class="field-label">{resolveLabel(f.label_key)}</span>{/if}
          <p class="note-text">{displayText(f)}</p>
          {#if f.desc_key}<p class="tip">{resolveLabel(f.desc_key)}</p>{/if}
        </div>
      {:else if f.type === 'multiline'}
        <label class="field">
          {#if f.label_key}<span class="field-label">{resolveLabel(f.label_key)}</span>{/if}
          <textarea
            rows="4"
            placeholder={resolveLabel(f.placeholder_key)}
            value={String(values[f.key] ?? '')}
            oninput={(e) => setText(f, e.currentTarget.value)}
          ></textarea>
        </label>
      {:else}
        <!-- text / secret / number -->
        <label class="field">
          {#if f.label_key}<span class="field-label">{resolveLabel(f.label_key)}</span>{/if}
          <input
            type={f.type === 'secret' ? 'password' : f.type === 'number' ? 'number' : 'text'}
            autocomplete={f.type === 'secret' ? 'new-password' : 'off'}
            placeholder={f.type === 'secret' ? secretPlaceholder(f) : resolveLabel(f.placeholder_key)}
            value={String(values[f.key] ?? '')}
            oninput={(e) => setText(f, e.currentTarget.value)}
          />
          {#if f.desc_key}<p class="tip">{resolveLabel(f.desc_key)}</p>{/if}
        </label>
      {/if}
    {/if}
  {/each}

  {#if error}
    <div class="error"><Icon name="alert" size={14} /> {error}</div>
  {/if}

  {#if section.actions?.length}
    <div class="actions">
      {#if saved}<span class="saved">{t('note.saved')}</span>{/if}
      {#each section.actions as action (action.id)}
        {#if evalShowIf(action.show_if, values)}
          <Button
            variant={action.variant ?? 'default'}
            label={saving && action.variant === 'primary' ? t('settings.connecting') : resolveLabel(action.label_key)}
            onclick={() => runAction(action)}
            disabled={saving || (action.variant === 'primary' && prefillMissing)}
          />
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .section {
    max-width: 520px;
  }
  /* 展示型字段（copy/note）：只读，视觉上与可编辑输入拉开距离 */
  .copy-row {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .copy-input {
    flex: 1 1 auto;
    min-width: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12px;
    background: var(--hover);
    cursor: text;
  }
  .note-text {
    margin: 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--text-dim);
    word-break: break-word;
  }
  h3 {
    margin: 0 0 4px;
    font-size: 16px;
  }
  .hint {
    color: var(--text-dim);
    font-size: 13px;
    margin: 4px 0 16px;
  }
  .field {
    display: block;
    margin-top: 16px;
  }
  .field.row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .field-label {
    display: block;
    font-size: 12px;
    color: var(--text-dim);
    margin-bottom: 6px;
  }
  .field.row .field-label {
    margin-bottom: 0;
  }
  input[type='text'],
  input[type='password'],
  input[type='number'],
  textarea {
    display: block;
    width: 100%;
    box-sizing: border-box;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--bg-side);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  input:focus,
  textarea:focus {
    outline: none;
    border-color: var(--accent);
  }
  .tip {
    font-size: 11px;
    color: var(--text-dim);
    margin: 6px 0 0;
  }
  .seg {
    display: flex;
    gap: 8px;
  }
  .seg-wrap {
    flex-wrap: wrap;
  }
  .seg-wrap button {
    flex: 1 1 calc(50% - 8px);
    min-width: 120px;
  }
  .seg button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 9px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-side);
    color: var(--text);
    cursor: pointer;
    font-size: 13px;
    transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  }
  .seg button:active {
    transform: scale(0.98);
  }
  .seg button.on {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }
  .toggle {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin-top: 18px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-side);
    cursor: pointer;
  }
  .toggle input {
    width: 16px;
    height: 16px;
    margin: 1px 0 0;
    flex: 0 0 auto;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .toggle-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .toggle-title {
    font-size: 13px;
    color: var(--text);
    font-weight: 600;
  }
  .toggle-desc {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .plugin-form {
    margin-top: 16px;
  }
  .folder-list {
    margin-top: 6px;
    max-height: 220px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px;
  }
  .folder-check {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 4px;
    font-size: 13px;
    cursor: pointer;
    border-radius: 6px;
  }
  .folder-check:hover {
    background: var(--hover, rgba(127, 127, 127, 0.1));
  }
  .folder-check input {
    flex: none;
  }
  .error {
    margin-top: 16px;
    padding: 8px 10px;
    background: var(--danger-soft);
    color: var(--danger);
    border-radius: 7px;
    font-size: 12px;
    word-break: break-all;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .actions {
    margin-top: 22px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
  }
  .actions .saved {
    color: var(--success);
    font-size: 12px;
  }
</style>
