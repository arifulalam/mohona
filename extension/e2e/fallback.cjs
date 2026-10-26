// End-to-end check: Mohona's host is NOT registered, so the browser must download the file itself.
// Run: H=<empty temp folder> HOME=$H node e2e/fallback.cjs (test server on 127.0.0.1:18080)
const { chromium } = require('playwright');
(async () => {
  const ext = process.cwd() + '/dist/chromium';
  const ctx = await chromium.launchPersistentContext(process.env.H + '/profile', {
    headless: true, channel: 'chromium', acceptDownloads: true,
    args: [`--disable-extensions-except=${ext}`, `--load-extension=${ext}`, '--headless=new'],
    env: { ...process.env, HOME: process.env.H },
  });
  let [sw] = ctx.serviceWorkers();
  if (!sw) sw = await ctx.waitForEvent('serviceworker', { timeout: 15000 });
  const web = await ctx.newPage();
  await web.goto('http://127.0.0.1:18080/');
  await web.evaluate(() => { const a = document.createElement('a'); a.href = '/file?size=30000&seed=3'; a.download = 'plain.bin'; document.body.append(a); a.click(); });
  let list = [];
  for (let i = 0; i < 80 && !list.some((d) => d.state === 'complete'); i++) {
    list = await sw.evaluate(() => chrome.downloads.search({}));
    await new Promise((r) => setTimeout(r, 250));
  }
  console.log('fallback: browser kept', list.map((d) => `${d.state} ${d.filename.split('/').pop()} ${d.fileSize}`).join(', '));
  await ctx.close();
  if (!list.some((d) => d.state === 'complete')) process.exit(1);
})().catch((e) => { console.error('FAIL', e.message); process.exit(1); });
