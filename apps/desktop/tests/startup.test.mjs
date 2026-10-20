// Startup uses actual scan readiness and keeps failures actionable (§6.1).
// These render checks exercise localized states without a native scan or delay.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { StartupScreen, I18nProvider } from './helpers.mjs';

function render(props, language = 'en') {
  globalThis.window = { localStorage: { getItem: () => language } };
  try { return renderToStaticMarkup(createElement(I18nProvider, null,
    createElement(StartupScreen, { leaving: false, preview: false, sessions: 0, agents: 0, ...props }))); }
  finally { delete globalThis.window; }
}

test('scanning startup offers background browsing without claiming completion', () => {
  const html = render({ loading: true, error: null });
  assert.match(html, /Getting to know your workspace/);
  assert.match(html, /Continue in background/);
  assert.doesNotMatch(html, /Ready when you are/);
  assert.match(html, /Reading metadata only/);
  assert.match(html, /\/brand\/agenttidy-icon.png/);
});
test('startup failure preserves retry, diagnostic access and escaped error details', () => {
  const html = render({ loading: false, error: '<probe failed>' });
  assert.match(html, /Try again/);
  assert.match(html, /Open Diagnostics/);
  assert.match(html, /&lt;probe failed&gt;/);
  assert.doesNotMatch(html, /Ready when you are/);
});
test('ready startup reports actual counts in both languages', () => {
  const props = { loading: false, error: null, agents: 3, sessions: 354, preview: true };
  assert.match(render(props), /3 installations. 354 sessions/);
  const zh = render(props, 'zh');
  assert.match(zh, /已识别 3 个安装、354 个会话/);
  assert.match(zh, /进入工作台/);
});
