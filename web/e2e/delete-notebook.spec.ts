import { test, expect } from '@playwright/test'
import { openApp } from './helpers'

// 删除笔记本（级联）：新建笔记本 → 在其中建一篇笔记 → 删掉笔记本，
// 笔记本连同其中的笔记一起从树/列表/服务端消失。
// 用例自建自删，不动 fixture 里的 'Notebook'（其余用例依赖它）。
test('deleting a notebook removes it and the notes inside it', async ({ page, request }) => {
	await openApp(page)
	await expect(page.locator('button.folder', { hasText: 'Notebook' })).toBeVisible()

	// 新建笔记本（名称走浏览器 prompt）→ 自动选中
	page.once('dialog', (d) => d.accept('Doomed'))
	await page.getByRole('button', { name: 'New notebook' }).click()
	const row = page.locator('.row', { has: page.locator('button.folder', { hasText: 'Doomed' }) })
	await expect(row).toBeVisible()

	// 在其中建一篇笔记
	await page.getByRole('button', { name: 'New note in this notebook' }).click()
	const note = page.locator('button.note', { hasText: 'New note' })
	await expect(note).toBeVisible()

	// 记下新建笔记本的 id（供删除后核对服务端）
	const folders = (await (await request.get('/api/folders')).json()) as { id: string; title: string }[]
	const doomed = folders.find((f) => f.title === 'Doomed')!
	expect(doomed).toBeTruthy()

	// 删除：行内删除按钮 → confirm 确认（含条数提示）
	let confirmText = ''
	page.once('dialog', (d) => {
		confirmText = d.message()
		void d.accept()
	})
	await row.hover()
	await row.getByRole('button', { name: 'Delete notebook' }).click()

	await expect(row).toHaveCount(0)
	await expect(note).toHaveCount(0)
	expect(confirmText).toContain('1 note')

	// 服务端也没了；fixture 的笔记本不受影响
	const after = (await (await request.get('/api/folders')).json()) as { id: string; title: string }[]
	expect(after.find((f) => f.id === doomed.id)).toBeUndefined()
	expect(after.find((f) => f.title === 'Notebook')).toBeTruthy()
})
