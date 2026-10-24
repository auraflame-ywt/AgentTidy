// Mac browsing/confirmation edge cases (Start.md §6 / §12). Test decisions
// and async ordering independently of WebView rendering or a running agent.
import test from 'node:test';
import assert from 'node:assert/strict';
import { cleanup, sessions, reasons } from './helpers.mjs';
const now = 1_800_000_000_000, day = 86_400_000;
const row = (id, provider, state, updated, size, cwd, title = null) => ({
  installation: `${provider}:default`, session: { id, provider, title,
    lifecycle: { state }, updated_at: updated, created_at: null,
    size: { logical_bytes: size }, project: cwd == null ? null : { cwd } },
});
const rows = [row('recent', 'workbuddy', 'archived', now - day, 40, '/项目/文档 空格'),
  row('old', 'codex', 'inactive', now - 31 * day, 100, '/projects/test', 'Searchable Title'),
  row('unknown', 'claude-code', 'unknown', null, 10, null)];

test('search, provider, lifecycle and time combine without dropping CJK paths', () => {
  const query = { ...sessions.DEFAULT_SESSION_QUERY, text: '文档 空格', provider: 'workbuddy',
    lifecycle: 'archived', period: 'week' };
  assert.deepEqual(sessions.querySessions(rows, query, now).map(row => row.session.id), ['recent']);
  assert.equal(sessions.querySessions(rows, { ...query, lifecycle: 'inactive' }, now).length, 0);
});
test('unknown activity is last in both date directions and excluded from age filters', () => {
  for (const order of ['newest', 'oldest']) {
    const sorted = sessions.querySessions(rows, { ...sessions.DEFAULT_SESSION_QUERY, order }, now);
    assert.equal(sorted.at(-1).session.id, 'unknown');
  }
  const old = sessions.querySessions(rows, { ...sessions.DEFAULT_SESSION_QUERY, period: 'older' }, now);
  assert.deepEqual(old.map(row => row.session.id), ['old']);
});
test('search includes titles and size ordering does not mutate scan data', () => {
  assert.equal(sessions.querySessions(rows, { ...sessions.DEFAULT_SESSION_QUERY, text: 'SEARCHABLE' }, now)[0].session.id, 'old');
  assert.deepEqual(sessions.querySessions(rows, { ...sessions.DEFAULT_SESSION_QUERY, order: 'largest' }, now)
    .map(row => row.session.id), ['old', 'recent', 'unknown']);
  assert.equal(rows[0].session.id, 'recent');
});
test('request gate prevents rapid duplicate clicks and ignores cancelled responses', () => {
  const requests = new cleanup.CleanupRequests();
  const first = requests.begin('preview');
  assert.equal(requests.begin('execute'), null);
  requests.cancel();
  const next = requests.begin('preview');
  assert.equal(requests.current(first), false);
  requests.finish(first);
  assert.equal(requests.busy(), true);
  requests.finish(next);
  const execute = requests.begin('execute');
  assert.equal(requests.begin('execute'), null);
  requests.finish(execute);
  assert.equal(requests.busy(), false);
});
const plan = { expires_at: now + 10, items: [
  { unit_id: 'ok', risk: 'review-required' }, { unit_id: 'blocked', risk: 'blocked' },
] };
test('expired, unreviewed, spent, blocked and stale selections cannot execute', () => {
  const ids = new Set(['ok']);
  assert.equal(cleanup.canExecute(plan, ids, [], true, false, now), true);
  assert.equal(cleanup.canExecute(plan, ids, [], true, false, now + 11), false);
  assert.equal(cleanup.canExecute(plan, ids, [], false, false, now), false);
  assert.equal(cleanup.canExecute(plan, ids, [], true, true, now), false);
  assert.equal(cleanup.canExecute(plan, new Set(['blocked']), [], true, false, now), false);
  assert.equal(cleanup.canExecute(plan, new Set(['fabricated']), [], true, false, now), false);
  assert.equal(cleanup.canExecute(plan, ids, [{ unit_id: 'ok', outcome: 'skipped' }], true, false, now), false);
  assert.equal(cleanup.canExecute(plan, new Set(), [], true, false, now), false);
});
test('revalidation removes stale and blocked rows from selection', () => {
  const selected = cleanup.selectableIds(plan, new Set(['ok', 'blocked']), [{ unit_id: 'ok', outcome: 'skipped' }]);
  assert.equal(selected.size, 0);
});

test('verified safety reasons are localized while unknown native failures keep their details', () => {
  assert.equal(reasons.cleanupReasonKey('agent is currently running (§16.2)'), 'cleanup.reason.running');
  assert.equal(reasons.cleanupReasonKey('scan reported an Error problem for this installation'), 'cleanup.reason.scanError');
  assert.equal(reasons.cleanupReasonKey('new unknown OS error'), null);
});

// Recovery and CLI admission remain separate from transient safety failures.
test('Codex admission explains recovery and CLI blockers before confirmation', () => {
  const recovery = 'provider safety validation failed: Codex cleanup disabled: complete session recovery has not been validated';
  assert.equal(reasons.cleanupReasonKey(recovery), 'cleanup.reason.codexRecovery');
  for (const cli of ['Codex CLI unavailable:', 'Codex CLI unsupported:']) {
    assert.equal(reasons.cleanupReasonKey(`${recovery}; ${cli} details`), 'cleanup.reason.codexRecoveryCli');
  }
});

test('provider filters isolate Pi and Antigravity in a mixed session list', () => {
  const sources = [...rows, row('pi-session', 'pi', 'unknown', now, 1, '/pi'),
    row('agy-session', 'antigravity', 'unknown', now, 2, null)];
  for (const [provider, id] of [['pi', 'pi-session'], ['antigravity', 'agy-session']]) {
    assert.deepEqual(sessions.querySessions(sources, { ...sessions.DEFAULT_SESSION_QUERY, provider }, now)
      .map(row => row.session.id), [id]);
  }
});
