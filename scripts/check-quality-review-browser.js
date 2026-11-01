// Isolated, offline browser interaction check. Never opens the user's browser profile.
const fs = require('fs');
const path = require('path');
const assert = require('assert/strict');
const { pathToFileURL } = require('url');
const { chromium } = require('playwright');

async function main() {
  const file = path.resolve(process.argv[2] || 'docs/quality-review/2026-10-29/issues.html');
  const output = path.resolve('build/quality-review-ui-check');
  fs.mkdirSync(output, { recursive: true });
  const browser = await chromium.launch({ headless:true, executablePath:process.env.OWO_TEST_BROWSER || 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', args:['--disable-background-networking','--disable-extensions'] });
  try {
    const context = await browser.newContext({ viewport:{width:1440,height:1000}, acceptDownloads:true });
    await context.route(/^https?:/, route => route.abort());
    const page = await context.newPage(), errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(pathToFileURL(file).href);
    await page.locator('.review-row').first().waitFor();
    const selectedCount = () => page.evaluate(() => selected.size);
    const review = id => page.evaluate(id => state(byId.get(id)), id);
    const ids = await page.evaluate(() => filtered().slice(0,3).map(issue => issue.id));
    await page.locator('.review-row input[type=checkbox]').first().check(); assert.equal(await selectedCount(),1);
    await page.keyboard.press('Space'); assert.equal((await review(ids[0])).status,'确认问题');
    await page.keyboard.press('Backspace'); assert.equal((await review(ids[0])).status,'待批阅');
    await page.locator('#clear-selection').click();
    await page.keyboard.press('ArrowDown'); await page.keyboard.press('Shift+ArrowDown');
    assert.equal(await selectedCount(),2);
    await page.keyboard.press('Space');
    for (const id of ids.slice(0,2)) assert.equal((await review(id)).status,'确认问题');
    await page.locator('.review-row').nth(2).hover(); await page.keyboard.press('z');
    assert.equal((await review(ids[2])).status,'可接受歧义');
    assert.equal((await review(ids[0])).status,'确认问题');
    await page.mouse.move(0,0); await page.keyboard.press('Backspace');
    for (const id of ids.slice(0,2)) assert.equal((await review(id)).status,'待批阅');
    await page.keyboard.press('ArrowRight'); assert.equal(await page.evaluate(() => page),1);
    await page.keyboard.press('Shift+ArrowUp'); assert.equal(await page.evaluate(() => page),0); assert.equal(await selectedCount(),2);
    const notes = page.locator('.review-row textarea').last();
    await notes.fill('keyboard guard'); await notes.press('Space'); assert.equal(await notes.inputValue(),'keyboard guard ');
    await notes.press('Backspace'); await notes.press('ArrowLeft'); assert.equal(await page.evaluate(() => page),0);
    await page.locator('#clear-selection').click(); await page.locator('#select-page').click();
    assert.equal(await selectedCount(),50);
    await page.locator('#bulk-status').selectOption('需要复现'); await page.locator('#bulk-notes').fill('浏览器批量批注测试');
    await page.locator('#bulk-apply').click();
    assert.ok(await page.evaluate(() => [...selected].every(id => state(byId.get(id)).status==='需要复现' && state(byId.get(id)).notes.includes('浏览器批量批注测试'))));
    await page.locator('#undo').click(); assert.equal((await review(ids[0])).status,'待批阅');
    await page.screenshot({path:path.join(output,'review-multi-selection.png')});
    let dialogText=''; page.on('dialog',async dialog => {dialogText=dialog.message();await dialog.accept();});
    const imported={schema_version:1,reviews:[{id:ids[0],status:'不复现',notes:'isolated browser import check'}]};
    await page.locator('#file').setInputFiles({name:'review.json',mimeType:'application/json',buffer:Buffer.from(JSON.stringify(imported))});
    await page.waitForFunction(id => state(byId.get(id)).status==='不复现',ids[0]);
    assert.ok(dialogText.includes('1'));
    const downloadPromise=page.waitForEvent('download'); await page.locator('#save').click(); const download=await downloadPromise;
    const stream=await download.createReadStream(), chunks=[];for await (const chunk of stream) chunks.push(chunk);
    const saved=JSON.parse(Buffer.concat(chunks).toString('utf8'));
    assert.equal(saved.reviews.find(issue=>issue.id===ids[0]).notes,'isolated browser import check');
    assert.deepEqual(errors,[]);
    console.log(JSON.stringify({ok:true,browser:'isolated headless Edge',checks:['keyboard range selection','space/z/backspace targeting','cross-page shift selection','textarea shortcut guards','bulk status/notes','undo','JSON import/download'],screenshot:path.join(output,'review-multi-selection.png'),scope:'fresh browser context; no real human annotations or browser profile modified'}));
  } finally {await browser.close();}
}
main().catch(error=>{console.error(error.stack);process.exitCode=1;});
