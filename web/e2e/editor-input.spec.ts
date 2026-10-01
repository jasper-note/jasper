import { test, expect, type APIRequestContext } from '@playwright/test'
import { openApp } from './helpers'
import { IDS } from './make-fixture.mjs'

// 编辑器输入细节：标题栏回车进正文、选中后键入符号包裹选区。
// 每个用例自建笔记、结束删掉——不碰 fixture 笔记（别的 spec 会改它们的标题/正文）。

async function makeNote(request: APIRequestContext, title: string, body: string): Promise<string> {
	const res = await request.post('/api/notes', { data: { parent_id: IDS.notebook, title, body } })
	expect(res.ok()).toBe(true)
	return (await res.json()).id
}

const bodyOf = async (request: APIRequestContext, id: string): Promise<string> =>
	(await (await request.get(`/api/notes/${id}`)).json()).body

test('Enter in the title moves the cursor to the start of the body', async ({ page, request }) => {
	const id = await makeNote(request, 'Title Enter', 'first line\nsecond line')
	await openApp(page, { editor: 'source' })
	await page.locator('button.note', { hasText: 'Title Enter' }).click()

	const cm = page.locator('.cm-content')
	await expect(cm).toBeVisible()
	// 先把正文光标放到末尾：回车后应落到正文开头，而不是回到原来的位置
	await cm.click()
	await page.keyboard.press('ControlOrMeta+End')

	const title = page.locator('input.title-input')
	await title.click()

	// 输入法组字中的回车（确认候选词）不跳
	await title.evaluate((el) =>
		el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true })),
	)
	await expect(title).toBeFocused()

	await page.keyboard.press('Enter')
	await expect(cm).toBeFocused()
	await page.keyboard.type('NEW ')

	await expect(title).toHaveValue('Title Enter')
	await expect.poll(() => bodyOf(request, id), { timeout: 3000 }).toBe('NEW first line\nsecond line')

	await request.delete(`/api/notes/${id}`)
})

test('typing a symbol over a selection wraps it', async ({ page, request }) => {
	const id = await makeNote(request, 'Wrap Symbols', 'wrap me')
	await openApp(page, { editor: 'source' })
	await page.locator('button.note', { hasText: 'Wrap Symbols' }).click()

	const cm = page.locator('.cm-content')
	await expect(cm).toBeVisible()
	await cm.click()
	await page.keyboard.press('ControlOrMeta+Home')
	for (let i = 0; i < 'wrap'.length; i++) await page.keyboard.press('Shift+ArrowRight')

	// 包裹后选区仍是原文字，可以连续套多层；右引号（输入法交替出字）同样按整对包裹
	await page.keyboard.type('（')
	await page.keyboard.type('`')
	await page.keyboard.type('”')

	await expect.poll(() => bodyOf(request, id), { timeout: 3000 }).toBe('（`“wrap”`） me')

	await request.delete(`/api/notes/${id}`)
})
