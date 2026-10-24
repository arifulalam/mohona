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
  // 5: a link the browser would download itself is taken over by Mohona.
  const link = `http://127.0.0.1:18080/file?size=70000&seed=7`;
  const web = await ctx.newPage();
  await web.goto('http://127.0.0.1:18080/');
  await web.evaluate((u) => { const a = document.createElement('a'); a.href = u; a.download = ''; document.body.append(a); a.click(); }, link);
  const started = Date.now();
  let got = '';
  while (Date.now() - started < 20000) {
    const text = await (async () => { await page.reload(); await new Promise(r => setTimeout(r, 700)); return page.textContent('#last'); })();
    if (text) { got = text; break; }
  }
  console.log('5:', got);
  const left = await sw.evaluate(() => chrome.downloads.search({}).then(d => d.length));
  console.log('6: downloads left in the browser:', left);
  // 7: media on a page is detected, and an HLS stream is downloaded by Mohona.
  await web.evaluate(() => fetch('/hls/master.m3u8').then((r) => r.text()));
  const tabId = await sw.evaluate(() => chrome.tabs.query({ url: 'http://127.0.0.1:18080/*' }).then((t) => t[0].id));
  let media = [];
  for (let i = 0; i < 40 && !media.length; i++) {
    media = (await page.evaluate((id) => chrome.runtime.sendMessage({ kind: 'media', tabId: id }), tabId)).media;
    if (!media.length) await new Promise((r) => setTimeout(r, 250));
  }
  console.log('7: detected', media.map((m) => m.kind + ' ' + m.name).join(', '));
  const outcome = (await page.evaluate((m) => chrome.runtime.sendMessage({ kind: 'downloadMedia', media: m }), media[0])).outcome;
  console.log('8:', JSON.stringify(outcome));
  await new Promise((r) => setTimeout(r, 6000));
  await ctx.close();
})().catch(e => { console.error('FAIL', e.message); process.exit(1); });
