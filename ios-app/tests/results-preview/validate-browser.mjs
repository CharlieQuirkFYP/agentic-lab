/** Browser helpers for the real screen. Import from the browser console or automation. */
export const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
export async function click(label) {
  const target = [...document.querySelectorAll('[aria-label]')].find(element => element.getAttribute('aria-label') === label);
  if (!target) throw new Error(`Missing UI action: ${label}`);
  target.click();
  await settle();
}
export async function fill(label, value) {
  const target = document.querySelector(`input[aria-label="${label}"]`);
  if (!target) throw new Error(`Missing input: ${label}`);
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(target, value);
  target.dispatchEvent(new Event('input', { bubbles: true }));
  await settle();
}
export function inspect() {
  const bounds = document.getElementById('screen').getBoundingClientRect();
  const overflow = [...document.querySelectorAll('#screen *')].filter(element => {
    if (!(element instanceof HTMLElement) || !element.offsetWidth) return false;
    const rect = element.getBoundingClientRect();
    return rect.left < bounds.left - 1 || rect.right > bounds.right + 1;
  }).map(element => ({ text: element.textContent?.slice(0, 100), label: element.getAttribute('aria-label'), width: element.offsetWidth }));
  return { viewport: { width: innerWidth, height: innerHeight }, url: location.href, overflow,
    text: document.getElementById('screen').innerText,
    buttons: [...document.querySelectorAll('#screen [role="button"]')].map(element => ({ label: element.getAttribute('aria-label') ?? element.textContent, height: element.getBoundingClientRect().height })),
    svgCount: document.querySelectorAll('#screen svg').length };
}
export function checkText(...expected) {
  const text = document.getElementById('screen').innerText;
  for (const value of expected) if (!text.includes(value)) throw new Error(`Expected displayed text: ${value}`);
}
export function captureShare() {
  const captured = [];
  Object.defineProperty(navigator, 'share', { configurable: true, value: async content => { captured.push(content); } });
  return captured;
}

/** Run from a fresh standard-fixture overview; returns reviewable DOM evidence. */
export async function runNavigation() {
  const shares = captureShare();
  const overview = inspect();
  checkText('17.35 s', '30 measured readings');
  await click('Share'); await click('Share summary'); await click('Export JSON');
  const exported = JSON.parse(window.__resultsPreviewExport.content);
  await click('Open App memory'); checkText('30 measured readings across 6 operations');
  const metric = inspect();
  await click('Inspect Turn 2'); checkText('10 measured readings', '3,056');
  const turn = inspect();
  await click('Reply generation, attempt 1'); checkText('5 measured readings', '6.4 s');
  const operation = inspect();
  await click('View readings and measurement details'); checkText('Saved readings', '5 total');
  const raw = inspect();
  await click('Back to all turns'); checkText('30 measured readings');
  await click('‹ Results'); await click('Share summary');
  if (shares.at(-1).text !== shares[0].text) throw new Error('Summary changed after detail navigation');
  await click('Choose conversation'); await fill('Filter saved results by model or conversation', 'saved');
  const filtered = inspect();
  const savedAction = filtered.buttons.find(item => item.label.startsWith('Select '));
  if (!savedAction) throw new Error('Saved conversation missing after search');
  await click(savedAction.label); checkText('13.65 s', '2 / 2');
  const saved = inspect();
  for (const [name, result] of Object.entries({ overview, metric, turn, operation, raw, saved })) {
    if (result.overflow.length) throw new Error(`Horizontal overflow in ${name}: ${JSON.stringify(result.overflow)}`);
  }
  return { passed: true, overview, metric, turn, operation, raw, filtered, saved, summary: shares[0].text,
    summaryUnchanged: true, exportedTotals: exported.totals };
}
