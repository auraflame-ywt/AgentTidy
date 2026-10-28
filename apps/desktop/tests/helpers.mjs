// Compile the pure TypeScript helpers with the installed compiler, then
// exercise them using Node's built-in runner (no DOM or new dependencies).
import { mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { after } from 'node:test';
const output = mkdtempSync(join(tmpdir(), 'agenttidy-ui-tests-'));
execFileSync(process.execPath, ['node_modules/typescript/bin/tsc', '--target', 'ES2020',
  '--module', 'commonjs', '--jsx', 'react-jsx', '--strict', '--skipLibCheck', '--outDir', output,
  'src/cleanup-flow.ts', 'src/session-query.ts', 'src/cleanup-reasons.ts', 'src/presentation.ts',
  'src/components/StartupScreen.tsx', 'src/views/Overview.tsx', 'src/views/Sessions.tsx', 'src/views/Review.tsx'], { cwd: new URL('..', import.meta.url) });
// Linking the workspace's `node_modules` into the compiled output lets the
// test require React and the other runtime deps. On Windows a `'dir'`
// symlink needs Developer Mode or administrator rights and fails with
// EPERM, so use a `'junction'` instead — junctions need no elevation.
// A junction target must be an absolute path, hence `resolve`.
const nodeModulesTarget = resolve(new URL('../node_modules', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'));
symlinkSync(nodeModulesTarget, join(output, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
after(() => rmSync(output, { recursive: true, force: true }));
const require = createRequire(import.meta.url);
export const cleanup = require(join(output, 'cleanup-flow.js'));
export const sessions = require(join(output, 'session-query.js'));

export const reasons = require(join(output, 'cleanup-reasons.js'));

export const presentation = require(join(output, 'presentation.js'));
export const Overview = require(join(output, 'views/Overview.js')).default;
export const Review = require(join(output, 'views/Review.js')).default;
export const SessionDetail = require(join(output, 'views/SessionDetail.js')).default;
export const ExecutionResults = require(join(output, 'views/ExecutionResults.js')).default;
export const I18nProvider = require(join(output, 'i18n/index.js')).I18nProvider;

export const StartupScreen = require(join(output, 'components/StartupScreen.js')).default;
