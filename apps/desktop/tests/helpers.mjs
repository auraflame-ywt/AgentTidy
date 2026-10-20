// Compile the pure TypeScript helpers with the installed compiler, then
// exercise them using Node's built-in runner (no DOM or new dependencies).
import { mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { after } from 'node:test';
const output = mkdtempSync(join(tmpdir(), 'agenttidy-ui-tests-'));
execFileSync(process.execPath, ['node_modules/typescript/bin/tsc', '--target', 'ES2020',
  '--module', 'commonjs', '--jsx', 'react-jsx', '--strict', '--skipLibCheck', '--outDir', output,
  'src/cleanup-flow.ts', 'src/session-query.ts', 'src/cleanup-reasons.ts', 'src/presentation.ts',
  'src/components/StartupScreen.tsx', 'src/views/Overview.tsx', 'src/views/Sessions.tsx', 'src/views/Review.tsx'], { cwd: new URL('..', import.meta.url) });
symlinkSync(new URL('../node_modules', import.meta.url), join(output, 'node_modules'), 'dir');
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
