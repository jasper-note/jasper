import { test, expect } from '@playwright/test'
import { openApp } from './helpers'
import { IDS } from './make-fixture.mjs'

// Cmd/Ctrl+S（浏览器默认 = 「保存网页」另存为对话框）：全页拦下默认行为，并立即冲刷当前笔记的
// 未保存改动，不等 800ms 防抖。断言分两半：
// ① 默认行为被拦（真对话框无法在 Playwright 里观测，用 defaultPrevented 代表）；
// ② 保存在按键当下发生——按 PUT 到达时刻与按键时刻的间隔判断，未冲刷时它必然落在防抖（800ms）之后。
test('Cmd/Ctrl+S is intercepted and flushes the pending save immediately', async ({ page, request }) => {
	await openApp(page, { editor: 'source' })
	await page.locator('button.note', { hasText: 'Plain Note' }).click()

	const cm = page.locator('.cm-content')
	await expect(cm).toBeVisible()

	// 探针监听晚于 App 的 window 监听注册 → 在它之后执行，看到的是已被 preventDefault 的事件。
	await page.evaluate(() => {
		;(window as unknown as { __saveKey: unknown }).__saveKey = null
		window.addEventListener('keydown', (e) => {
			if (e.key.toLowerCase() === 's' && (e.metaKey || e.ctrlKey)) {
				;(window as unknown as { __saveKey: unknown }).__saveKey = { prevented: e.defaultPrevented }
			}
		})
	})

	let putAt = 0
	await page.route(`**/api/notes/${IDS.plainNote}`, async (route) => {
		if (route.request().method() !== 'PUT') return route.continue()
		putAt = Date.now()
		return route.continue()
	})

	// 敲入改动（防抖 800ms 内不会自己保存），随即按快捷键
	await cm.click()
	await page.keyboard.press('End')
	await page.keyboard.type(' SAVED-BY-SHORTCUT')
	const pressedAt = Date.now()
	await page.keyboard.press('Control+s')

	await expect
		.poll(() => page.evaluate(() => (window as unknown as { __saveKey: unknown }).__saveKey))
		.toEqual({ prevented: true })

	await expect
		.poll(async () => (await (await request.get(`/api/notes/${IDS.plainNote}`)).json()).body, {
			timeout: 3000,
		})
		.toContain('SAVED-BY-SHORTCUT')

	expect(putAt).toBeGreaterThan(0)
	expect(putAt - pressedAt).toBeLessThan(500) // 未冲刷则 ≈800ms（防抖）

	await page.unroute(`**/api/notes/${IDS.plainNote}`)
})