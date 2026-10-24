// End-to-end check in a real Chromium (Playwright): extension -> mohona-host -> mohona daemon.
// Needs: cargo build -p mohona-cli -p mohona-host; npm run build; a temporary HOME with the host
// registered (see docs/manual-tests.md). Run: HOME=$H XDG_DATA_HOME=$H/data node e2e/chromium.cjs
// with H set to that folder; its profile/NativeMessagingHosts must hold the manifest.
const { chromium } = require('playwright');
(async () => {
  const ext = process.cwd() + '/dist/chromium';
  const ctx = await chromium.launchPersistentContext(process.env.H + '/profile', {
    headless: true, channel: 'chromium', args: [`--disable-extensions-except=${ext}`, `--load-extension=${ext}`, '--headless=new'],
    env: { ...process.env, HOME: process.env.H },
  });
  let [sw] = ctx.serviceWorkers();
  if (!sw) sw = await ctx.waitForEvent('serviceworker', {timeout: 15000});
  console.log('worker', sw.url());
  const id = sw.url().split('/')[2];
  const page = await ctx.newPage();
  await page.goto(`chrome-extension://${id}/popup.html`);
  await page.waitForFunction(() => document.getElementById('status').textContent !== 'Checking…', null, {timeout: 15000});
  console.log('1:', await page.textContent('#status'));
  await page.click('#start');
  await page.waitForFunction(() => /is running/.test(document.getElementById('status').textContent), null, {timeout: 20000});
  console.log('2:', await page.textContent('#status'));
  await page.fill('#url', 'https://example.com/a.zip'); await page.click('#add');
  await page.waitForFunction(() => document.getElementById('result').textContent, null, {timeout: 15000});
  console.log('3:', await page.textContent('#result'));
  await page.fill('#url', 'https://example.com/a.zip'); await page.click('#add');
  await page.waitForFunction(() => /Not added/.test(document.getElementById('result').textContent), null, {timeout: 15000});
  console.log('4:', await page.textContent('#result'));
  await ctx.close();
})().catch(e => { console.error('FAIL', e.message); process.exit(1); });
