// Decision-flow regressions (Start.md §6 / §12): joins must never guess
// ownership, hide inspection diagnostics or report unsubmitted skips as results.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { presentation as view, Overview, SessionDetail, ExecutionResults, I18nProvider } from './helpers.mjs';

const size = { logical_bytes: 4096, allocated_bytes: null, exclusive_bytes: 4096,
  shared_bytes: null, reclaimable_bytes: null, confidence: 'exact' };
const session = { id: 'session-1', title: 'Repair <rendering>', installation_id: 'codex:default', provider: 'codex',
  project: { cwd: '/项目/演示', display_name: null }, created_at: 1, updated_at: 2,
  lifecycle: { state: 'archived', archived_at: 2 }, size, resource_refs: [{ id: 'r', kind: 'session' }] };
const resource = { id: 'r', provider: 'codex', installation_id: 'codex:default', kind: 'session',
  session_id: session.id, locator: { type: 'file', path: '/state/a.jsonl' }, size,
  ownership: 'exclusive', managed_by: 'agent', metadata: {}, dependencies: [] };
const installation = { id: 'codex:default', provider: 'codex', data_roots: ['/state'], status: 'available', platform: 'macos' };
const snapshot = { installation, sessions: [session], resources: [resource], completed_at: 3, problems: { shared: {} } };
const item = { unit_id: 'opaque-unit', installation_id: installation.id, provider: 'codex',
  fingerprint: { path: '/state/db.sqlite' }, locator: { kind: 'db-row', database: '/state/db.sqlite', record_id: session.id },
  evidence: { members: [{ path: '/state/a.jsonl' }] }, reasons: ['Codex cleanup disabled: complete session recovery has not been validated'],
  risk: 'blocked', action: 'provider-operation', estimated_reclaimable_bytes: 4096 };
const report = { installation, inspection: { agent_running: true, problems: { shared: {}, inspectionOnly: {} } } };
const plan = { items: [item], expires_at: Date.now() + 60_000, total_reclaimable_bytes: 0, risk_summary: { blocked: 1 } };
const render = (Component, props, lang = 'en') => {
  globalThis.window = { localStorage: { getItem: () => lang } };
  try { return renderToStaticMarkup(createElement(I18nProvider, null, createElement(Component, props))); }
  finally { delete globalThis.window; }
};

test('cleanup identity requires an unambiguous resource link in the same installation', () => {
  assert.equal(view.cleanupSession(item, [snapshot]), session);
  assert.equal(view.cleanupSession({ ...item, installation_id: 'another' }, [snapshot]), null);
  assert.equal(view.cleanupSession({ ...item, evidence: { members: [{ path: '/state/db.sqlite' }] } }, [snapshot]), null);
  const ambiguous = { ...snapshot, sessions: [session, { ...session, id: 'other', resource_refs: [{ id: 'other-r' }] }],
    resources: [resource, { ...resource, id: 'other-r', session_id: 'other' }] };
  assert.equal(view.cleanupSession(item, [ambiguous]), null);
  assert.deepEqual(view.sessionCleanupItems(snapshot, session, plan), [item]);
});
test('overview combines inspection and scan diagnostics without double-counting', () => {
  assert.equal(view.diagnosticCount(report, snapshot), 2);
  assert.deepEqual(view.storageSummary([snapshot]), { bytes: 4096, sessions: 1, estimated: false, scannedAt: 3 });
  assert.equal(view.storageSummary([{ ...snapshot, resources: [{ ...resource, size: { ...size, confidence: 'unknown' } }] }]).estimated, true);
  assert.equal(view.storageSummary([]).scannedAt, null);
});
test('overview and blocked detail render actionable, localized metadata without raw HTML', () => {
  const html = render(Overview, { reports: [report], snapshots: [snapshot], plan, loading: false });
  assert.match(html, /Explore largest sessions/);
  assert.match(html, /2 diagnostics/);
  const detail = render(SessionDetail, { snapshot, session, plan });
  assert.match(detail, /Repair &lt;rendering&gt;/);
  assert.match(detail, /Codex cleanup is disabled/);
  const zh = render(SessionDetail, { snapshot, session, plan }, 'zh');
  assert.match(zh, /仅展示元数据/);
  assert.match(zh, /完整会话恢复尚未验证/);
});
test('receipt excludes unselected items and shows original locations with manual recovery guidance', () => {
  const trash = { ...item, action: 'trash', locator: { kind: 'file-set', paths: ['/state/a.jsonl', '/state/a'] } };
  const html = render(ExecutionResults, { items: [trash], snapshots: [snapshot], attemptedIds: new Set([item.unit_id]), outcomes: [
    { unit_id: item.unit_id, outcome: 'failed', reason: 'partially moved' },
    { unit_id: 'never-submitted', outcome: 'skipped', reason: 'user did not select this unit' },
  ] });
  assert.match(html, /0 completed · 0 skipped · 1 failed/);
  assert.match(html, /failed item may have moved only some files/);
  assert.match(html, /\/state\/a.jsonl/);
  assert.doesNotMatch(html, /never-submitted/);
  assert.match(html, /Put Back/);
});

test('review leads with the lifecycle decision and preserves hard blockers', () => {
  const reasons = ['kind is FileSet (trash-first eligible)', 'agent is not running', 'session abc is Inactive (§11.2)'];
  assert.equal(view.primaryCleanupReason({ ...item, risk: 'review-required', reasons }), reasons[2]);
  assert.equal(view.primaryCleanupReason({ ...item, risk: 'blocked', reasons: ['writer lock present', ...reasons] }), 'writer lock present');
});

test('a missing execution response retains recovery references without inventing success counts', () => {
  const html = render(ExecutionResults, { items: [item], snapshots: [snapshot], attemptedIds: new Set([item.unit_id]), outcomes: null });
  assert.match(html, /complete result was not received/);
  assert.match(html, /Verify result/);
  assert.match(html, /\/state\/db.sqlite/);
  assert.doesNotMatch(html, /0 completed/);
});

// New metadata-only sources must never inherit WorkBuddy labels or cleanup UI.
test('Antigravity and Pi render their own labels and retain multiple workspaces', () => {
  for (const [provider, label] of [['antigravity', 'Antigravity'], ['pi', 'Pi']]) {
    const next = { ...session, provider, lifecycle: { state: 'unknown' }, project: null,
      metadata: { workspace_uris: ['file:///Users/sample/a', 'file:///Users/sample/b'] } };
    const nextSnapshot = { ...snapshot, installation: { ...installation, provider }, sessions: [next], resources: [] };
    const html = render(SessionDetail, { snapshot: nextSnapshot, session: next, plan: { ...plan, items: [] } });
    assert.match(html, new RegExp(label));
    assert.match(html, /Associated workspaces \(2\)/);
    assert.match(html, /file:\/\/\/Users\/sample\/a/);
    assert.doesNotMatch(html, />WorkBuddy</);
    assert.equal(view.sessionCleanupItems(nextSnapshot, next, { ...plan, items: [] }).length, 0);
  }
});
