// Auxiliary evidence helper (playtester, write scope: playtest/).
// Records the *actual* browser channel + version the live playtest used.
// Not a replacement for live-playtest.mjs; it only fills the browser-fingerprint gap.
const { chromium } = require('playwright');
const fs = require('node:fs');

(async () => {
  const out = { channel: 'msedge', launched: false };
  let browser;
  try {
    browser = await chromium.launch({ channel: 'msedge', headless: true });
    out.launched = true;
    out.engine_version = browser.version();
    const context = await browser.newContext();
    const page = await context.newPage();
    out.user_agent = await page.evaluate(() => navigator.userAgent);
    out.user_agent_data = await page.evaluate(() => (navigator.userAgentData ? {
      brands: navigator.userAgentData.brands,
      platform: navigator.userAgentData.platform,
      mobile: navigator.userAgentData.mobile,
    } : null));
  } catch (err) {
    out.error = String(err && err.message || err);
  } finally {
    if (browser) await browser.close().catch(() => {});
  }
  out.captured_at = new Date().toISOString();
  fs.writeFileSync('playtest/browser-fingerprint.json', JSON.stringify(out, null, 2));
  console.log(JSON.stringify(out));
})();
