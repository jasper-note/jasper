#!/usr/bin/env python3
"""生成主题相关 CSS：
  - src/icons.css     基础图标令牌 --icon-<名>（SVG data URI，整段 URL 编码以过 lightningcss）
  - src/themes/*.css  内置示例主题（:root[data-theme='<id>'] 覆盖颜色语义令牌 + 部分图标）

新增/修改图标或主题改这里后重跑：  cd web && python3 scripts/gen-theme-assets.py
勿手改生成出的编码串。
"""
from urllib.parse import quote
import os

WRAP = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" '
        'stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{}</svg>')


def url(inner: str) -> str:
	return 'url("data:image/svg+xml,' + quote(WRAP.format(inner), safe='') + '")'


# ---------- 基础图标集 ----------
ICONS = {
	'close': '<path d="M18 6 6 18M6 6l12 12"/>',
	'plus': '<path d="M12 5v14M5 12h14"/>',
	'settings': '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>',
	'image': '<rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><path d="m21 15-5-5L5 21"/>',
	'folder': '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>',
	'file': '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6"/>',
	'alert': '<path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
	'edit': '<path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z"/>',
	'trash': '<path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
	'eye': '<path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>',
	'code': '<path d="m16 18 6-6-6-6M8 6l-6 6 6 6"/>',
	'rich': '<path d="m12 3-1.9 5.8a2 2 0 0 1-1.3 1.3L3 12l5.8 1.9a2 2 0 0 1 1.3 1.3L12 21l1.9-5.8a2 2 0 0 1 1.3-1.3L21 12l-5.8-1.9a2 2 0 0 1-1.3-1.3z"/>',
	'attach': '<path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/>',
	'clean': '<path d="m7 21-4.3-4.3a1 1 0 0 1 0-1.4l9.6-9.6a1 1 0 0 1 1.4 0l5.6 5.6a1 1 0 0 1 0 1.4L13 21"/><path d="M22 21H7"/><path d="m5 11 9 9"/>',
	'cloud': '<path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/>',
	'globe': '<circle cx="12" cy="12" r="10"/><path d="M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>',
	'sun': '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
	'moon': '<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>',
	'contrast': '<circle cx="12" cy="12" r="9"/><path d="M12 3v18a9 9 0 0 0 0-18z" fill="#000" stroke="none"/>',
	'palette': '<circle cx="13.5" cy="6.5" r=".6" fill="#000" stroke="none"/><circle cx="17.5" cy="10.5" r=".6" fill="#000" stroke="none"/><circle cx="8.5" cy="7.5" r=".6" fill="#000" stroke="none"/><circle cx="6.5" cy="12.5" r=".6" fill="#000" stroke="none"/><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10c.93 0 1.69-.76 1.69-1.69 0-.44-.18-.83-.44-1.13-.27-.3-.44-.69-.44-1.13A1.69 1.69 0 0 1 14.19 16H16c2.76 0 5-2.24 5-5 0-4.42-4.04-8-9-8z"/>',
	# 新建笔记本（文件夹 + 加号）
	'folder-plus': '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/><line x1="12" y1="11" x2="12" y2="17"/><line x1="9" y1="14" x2="15" y2="14"/>',
	# 新建待办（带勾方框）
	'check-square': '<polyline points="9 11 12 14 22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>',
	# 任务进度勾
	'check': '<polyline points="20 6 9 17 4 12"/>',
	# 插件（电源插头，Lucide plug）
	'plug': '<path d="M12 22v-5"/><path d="M9 8V2"/><path d="M15 8V2"/><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8Z"/>',
	# 访问控制（Lucide lock / lock-open）：解锁登录 / 锁定登出
	'lock': '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
	'unlock': '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/>',
	# ---- 编辑器格式化工具（markdown 工具栏，Lucide 线性风格）----
	'heading-1': '<path d="M4 12h8"/><path d="M4 18V6"/><path d="M12 18V6"/><path d="m17 12 3-2v8"/>',
	'heading-2': '<path d="M4 12h8"/><path d="M4 18V6"/><path d="M12 18V6"/><path d="M21 18h-4c0-4 4-3 4-6 0-1.5-2-2.5-4-1"/>',
	'bold': '<path d="M14 12a4 4 0 0 0 0-8H6v8"/><path d="M15 20a4 4 0 0 0 0-8H6v8Z"/>',
	'italic': '<line x1="19" y1="4" x2="10" y2="4"/><line x1="14" y1="20" x2="5" y2="20"/><line x1="15" y1="4" x2="9" y2="20"/>',
	'strikethrough': '<path d="M16 4H9a3 3 0 0 0-2.83 4"/><path d="M14 12a4 4 0 0 1 0 8H6"/><line x1="4" y1="12" x2="20" y2="12"/>',
	'quote': '<path d="M17 6H3"/><path d="M21 12H8"/><path d="M21 18H8"/><path d="M3 12v6"/>',
	'list': '<line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/>',
	'list-ordered': '<line x1="10" y1="6" x2="21" y2="6"/><line x1="10" y1="12" x2="21" y2="12"/><line x1="10" y1="18" x2="21" y2="18"/><path d="M4 6h1v4"/><path d="M4 10h2"/><path d="M6 18H4c0-1 2-2 2-3s-1-1.5-2-1"/>',
	'list-todo': '<rect x="3" y="5" width="6" height="6" rx="1"/><path d="m3 17 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/>',
	'link': '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>',
	'braces': '<path d="M8 3H7a2 2 0 0 0-2 2v5a2 2 0 0 1-2 2 2 2 0 0 1 2 2v5a2 2 0 0 0 2 2h1"/><path d="M16 3h1a2 2 0 0 1 2 2v5a2 2 0 0 1 2 2 2 2 0 0 1-2 2v5a2 2 0 0 1-2 2h-1"/>',
	'table': '<path d="M12 3v18"/><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/>',
	'minus': '<path d="M5 12h14"/>',
	'chevron-up': '<path d="m18 15-6-6-6 6"/>',
	'chevron-down': '<path d="m6 9 6 6 6-6"/>',
	'download': '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/>',
	'maximize': '<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>',
	'align-justify': '<line x1="3" x2="21" y1="6" y2="6"/><line x1="3" x2="21" y1="12" y2="12"/><line x1="3" x2="21" y1="18" y2="18"/>',
	'align-center': '<line x1="21" x2="3" y1="6" y2="6"/><line x1="17" x2="7" y1="12" y2="12"/><line x1="19" x2="5" y1="18" y2="18"/>',
	'chevrons-down-up': '<path d="m7 20 5-5 5 5"/><path d="m7 4 5 5 5-5"/>',
	'chevrons-up-down': '<path d="m7 15 5 5 5-5"/><path d="m7 9 5-5 5 5"/>',
	'sparkles': '<path d="M9.94 15.5A2 2 0 0 0 8.5 14.06l-6.14-1.58a.5.5 0 0 1 0-.96L8.5 9.94A2 2 0 0 0 9.94 8.5l1.58-6.14a.5.5 0 0 1 .96 0L14.06 8.5A2 2 0 0 0 15.5 9.94l6.14 1.58a.5.5 0 0 1 0 .96L15.5 14.06a2 2 0 0 0-1.44 1.44l-1.58 6.14a.5.5 0 0 1-.96 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/><path d="M4 17v2"/><path d="M5 18H3"/>',
	# 插件侧栏 / chat widget（spec 0.3 §9.1 新增令牌）
	'chat': '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>',
	'send': '<path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/>',
	'bot': '<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>',
	# 标签视图（侧栏标签区入口）
	'tag': '<path d="M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z"/><circle cx="7.5" cy="7.5" r=".5" fill="#000"/>',
	# 设置页展示型字段：复制 / 隐藏密钥（与 eye 成对）/ 重新生成（Lucide copy、eye-off、refresh-cw）
	'copy': '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
	'eye-off': '<path d="M9.88 9.88a3 3 0 1 0 4.24 4.24"/><path d="M10.73 5.08A10.43 10.43 0 0 1 12 5c7 0 10 7 10 7a13.16 13.16 0 0 1-1.67 2.68"/><path d="M6.61 6.61A13.526 13.526 0 0 0 2 12s3 7 10 7a9.74 9.74 0 0 0 5.39-1.61"/><line x1="2" x2="22" y1="2" y2="22"/>',
	'refresh': '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
}

# 主题里要覆盖的额外图标（不进基础集）
THEME_ICONS = {
	'sliders': '<line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/>',
	'book': '<path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/>',
}

# ---------- 内置示例主题 ----------
THEMES = [
	{
		'id': 'nord', 'name': 'Nord', 'base': 'dark',
		'colors': {
			'--bg': '#2e3440', '--bg-bar': '#3b4252', '--bg-side': '#2e3440',
			'--text': '#e5e9f0', '--text-dim': '#9aa6bd', '--border': '#434c5e',
			'--hover': '#3b4252', '--accent': '#88c0d0',
			'--accent-soft': 'rgba(136, 192, 208, 0.18)', '--code-bg': '#3b4252',
			'--danger': '#bf616a', '--danger-soft': 'rgba(191, 97, 106, 0.18)',
			'--danger-soft-weak': 'rgba(191, 97, 106, 0.08)', '--success': '#a3be8c',
			'--on-accent': '#2e3440', '--code-block-bg': '#272c36', '--code-block-text': '#d8dee9',
		},
		# 演示：换图标（设置 → 滑块）
		'icons': {'--icon-settings': 'sliders'},
	},
	{
		'id': 'solarized', 'name': 'Solarized Light', 'base': 'light',
		'colors': {
			'--bg': '#fdf6e3', '--bg-bar': '#eee8d5', '--bg-side': '#f5efdc',
			'--text': '#073642', '--text-dim': '#93a1a1', '--border': '#e3dcc4',
			'--hover': '#eee8d5', '--accent': '#268bd2',
			'--accent-soft': 'rgba(38, 139, 210, 0.14)', '--code-bg': '#eee8d5',
			'--danger': '#dc322f', '--danger-soft': 'rgba(220, 50, 47, 0.14)',
			'--danger-soft-weak': 'rgba(220, 50, 47, 0.06)', '--success': '#859900',
			'--on-accent': '#fdf6e3', '--code-block-bg': '#002b36', '--code-block-text': '#93a1a1',
		},
		# 演示：换图标（笔记本 → 书）
		'icons': {'--icon-folder': 'book'},
	},
]

here = os.path.dirname(os.path.abspath(__file__))
src = os.path.normpath(os.path.join(here, '..', 'src'))

# ---- icons.css ----
lines = [
	"/* 图标令牌：--icon-<名>（内联 SVG data URI，整段 URL 编码以过 lightningcss minify）。",
	"   由 Icon.svelte 经 CSS mask + currentColor 渲染 → 单色、跟随文字色、主题可覆盖。",
	"   ⚠️ 本文件由 scripts/gen-theme-assets.py 生成，勿手改。 */",
	":root {",
]
for name, inner in ICONS.items():
	lines.append(f"\t--icon-{name}: {url(inner)};")
lines.append("}")
open(os.path.join(src, 'icons.css'), 'w', encoding='utf-8').write("\n".join(lines) + "\n")

# ---- themes/*.css ----
os.makedirs(os.path.join(src, 'themes'), exist_ok=True)
for th in THEMES:
	out = [
		f"/* 示例插件主题「{th['name']}」（base: {th['base']}）。",
		"   覆盖颜色语义令牌 + 演示换图标。由 scripts/gen-theme-assets.py 生成，勿手改。",
		"   将来插件主题即一份这样的 .css（由插件宿主动态加载，而非现在的打包内置）。 */",
		f":root[data-theme='{th['id']}'] {{",
	]
	for k, v in th['colors'].items():
		out.append(f"\t{k}: {v};")
	for k, icon in th['icons'].items():
		out.append(f"\t{k}: {url(THEME_ICONS[icon])};")
	out.append("}")
	open(os.path.join(src, 'themes', f"{th['id']}.css"), 'w', encoding='utf-8').write("\n".join(out) + "\n")

print(f"icons.css: {len(ICONS)} icons; themes: {', '.join(t['id'] for t in THEMES)}")
