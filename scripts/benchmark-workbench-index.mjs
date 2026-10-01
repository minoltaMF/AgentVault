#!/usr/bin/env node
// Synthetic, isolated WebUI API benchmark. No user sessions are read or removed.
// Example: node scripts/benchmark-workbench-index.mjs --cli /path/to/release/cc-sessions --sessions 1000 --messages 100 --body-length 1024 --output /path/to/result.json
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { setTimeout as delay } from 'node:timers/promises';

const options = {};
for (let i = 2; i < process.argv.length; i += 2) {
  const key = process.argv[i];
  assert(['--cli', '--sessions', '--messages', '--body-length', '--output', '--timeout-ms', '--providers'].includes(key), `Unknown option: ${key}`);
  assert(process.argv[i + 1], `Missing value for ${key}`);
  options[key.slice(2)] = process.argv[i + 1];
}
assert(options.cli && options.output, '--cli and --output are required');
const cli = path.resolve(options.cli), output = path.resolve(options.output);
const count = Number(options.sessions ?? 1000), messages = Number(options.messages ?? 100), bodyLength = Number(options['body-length'] ?? 1024);
const timeout = Number(options['timeout-ms'] ?? 1800000);
for (const [key, value] of Object.entries({ count, messages, bodyLength, timeout })) assert(Number.isSafeInteger(value) && value > 0, `${key} must be a positive integer`);
assert(messages >= 2 && bodyLength >= 64, 'Use at least 2 messages and 64 body characters');
assert.equal(messages % 2, 0, '--messages must be even to generate complete user/assistant pairs');
await fs.access(cli);
const root = await fs.mkdtemp(path.join(os.tmpdir(), 'agentvault-index-benchmark-'));
const marker = 'AVBENCH_SPARSE_MATCH', changedMarker = 'AVBENCH_CHANGED_MATCH', userMarker = 'AVBENCH_USER_MATCH';
const providers = (options.providers ?? 'qoder,workbuddy,grok,pi').split(',');
assert(providers.length > 0 && new Set(providers).size === providers.length && providers.every(provider => ['qoder', 'workbuddy', 'grok', 'pi'].includes(provider)), '--providers must contain unique supported provider names');
const hash = async file => { const digest = createHash('sha256'); for await (const chunk of createReadStream(file)) digest.update(chunk); return digest.digest('hex'); };
const report = {
  schema_version: 2, started_at: new Date().toISOString(), fixture_root: root,
  machine: { platform: process.platform, arch: process.arch, os_release: os.release(), cpu: os.cpus()[0]?.model, logical_cpus: os.cpus().length, total_memory_bytes: os.totalmem(), node: process.version },
  cli: { path: cli, sha256: await hash(cli), build_profile: /[\\/]release[\\/]/.test(cli) ? 'release-path (caller must verify build)' : 'unspecified' },
  sample: { sessions_per_provider: count, messages_per_session: messages, expected_conversation_messages_per_session: messages, expected_user_messages_per_session: Math.ceil(messages / 2), expected_assistant_messages_per_session: Math.floor(messages / 2), body_characters: bodyLength },
  methodology: 'Synthetic repetitive ASCII text, not representative real-world language or compression ratios. Sequential providers; fresh application index, OS filesystem cache not flushed. Timings include HTTP start/poll overhead (25 ms polling). Restart retains the derived index. Sparse query matches one session; absent query matches none. Fixtures and logs are retained, never deleted.',
  providers: [], pass: false,
};
let child;
async function stop() {
  if (!child) return;
  const owned = child; child = undefined;
  if (owned.exitCode !== null || owned.signalCode !== null) return;
  const exited = once(owned, 'exit');
  owned.kill();
  await Promise.race([exited, delay(10000, undefined, { ref: false }).then(() => { if (owned.exitCode === null && owned.signalCode === null) owned.kill('SIGKILL'); })]);
  if (owned.exitCode === null && owned.signalCode === null) await exited;
}
process.on('SIGINT', () => { void stop().finally(() => process.exit(130)); });
process.on('SIGTERM', () => { void stop().finally(() => process.exit(143)); });
async function start(directory, settings) {
  const listener = net.createServer(); listener.listen(0, '127.0.0.1'); await once(listener, 'listening');
  const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
  const origin = `http://127.0.0.1:${port}`;
  child = spawn(cli, ['webui', '--host', '127.0.0.1', '--port', String(port)], {
    cwd: directory, windowsHide: true,
    env: { ...process.env, CC_SESSIONS_WEBUI_SETTINGS: settings, CC_SESSIONS_WEBUI_DIST: path.join(root, 'dist') },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let startupError; child.on('error', error => { startupError = error; });
  const logs = []; child.stdout.on('data', data => logs.push(data)); child.stderr.on('data', data => logs.push(data));
  child.on('close', () => { void fs.appendFile(path.join(directory, 'server.log'), Buffer.concat(logs)); });
  const deadline = performance.now() + 30000;
  while (performance.now() < deadline) {
    if (startupError) throw startupError;
    if (child.exitCode !== null) throw new Error('WebUI exited during startup');
    try {
      const html = await (await fetch(origin, { signal: AbortSignal.timeout(1000) })).text();
      const config = html.match(/window\.__CC_SESSIONS_WEBUI__ = (\{.*?\});/);
      if (config) {
        const token = JSON.parse(config[1]).apiToken;
        return async (command, args = {}) => {
          const response = await fetch(`${origin}/api/invoke/${command}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-CC-Sessions-Webui-Token': token }, body: JSON.stringify(args), signal: AbortSignal.timeout(timeout) });
          const value = await response.json();
          assert(response.ok, `${command}: ${JSON.stringify(value)}`); return value;
        };
      }
    } catch (error) { if (startupError) throw startupError; }
    await delay(50);
  }
  throw new Error('WebUI startup timed out');
}
async function poll(api, command, jobId) {
  const deadline = performance.now() + timeout;
  while (performance.now() < deadline) {
    const status = await api(command, { jobId });
    if (status.state !== 'running') {
      assert.equal(status.state, 'completed', JSON.stringify(status));
      assert.equal(status.failed_files, 0, JSON.stringify(status)); assert.equal(status.error, null);
      return status;
    }
    await delay(25);
  }
  throw new Error(`${command} timed out`);
}
async function inventory(directory) {
  const files = [];
  for (const item of await fs.readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, item.name);
    if (item.isDirectory()) files.push(...await inventory(file));
    else files.push({ path: file, bytes: (await fs.stat(file)).size, sha256: await hash(file) });
  }
  return files.sort((a, b) => a.path.localeCompare(b.path));
}
function rows(provider, id) {
  const records = [], timestamp = '2026-09-30T00:00:00Z';
  if (provider === 'pi') records.push({ type: 'session', version: 3, id, cwd: '/benchmark', timestamp });
  if (provider === 'workbuddy') records.push({ type: 'ai-title', aiTitle: id, sessionId: id, cwd: '/benchmark', timestamp: 1790726400000 });
  for (let n = 0; n < messages; n++) {
    const role = n % 2 ? 'assistant' : 'user';
    const lastAssistant = messages % 2 === 0 ? messages - 1 : messages - 2;
    const prefix = id === 's000000' && n === 0 ? userMarker : id === 's000000' && n === lastAssistant ? marker : `message-${n}`;
    const text = prefix + ' x'.repeat(Math.ceil(bodyLength / 2)).slice(0, bodyLength - prefix.length);
    if (provider === 'qoder') records.push({ type: role, uuid: `${id}-${n}`, parentUuid: n ? `${id}-${n - 1}` : '', sessionId: id, cwd: '/benchmark', timestamp, isMeta: false, message: { role, content: [{ type: 'text', text }] } });
    if (provider === 'workbuddy') records.push({ type: 'message', sessionId: id, role, timestamp: 1790726400000 + n, content: [{ type: role === 'user' ? 'input_text' : 'output_text', text: role === 'user' ? `<user_query>${text}</user_query>` : text }] });
    if (provider === 'grok') records.push({ timestamp: n + 1, method: 'session/update', params: { sessionId: id, update: { sessionUpdate: role === 'user' ? 'user_message_chunk' : 'agent_message_chunk', _meta: { promptIndex: Math.floor(n / 2) }, content: { type: 'text', text } } } });
    if (provider === 'pi') records.push({ type: 'message', id: `${id}-${n}`, parentId: n ? `${id}-${n - 1}` : null, timestamp, message: { role, content: text } });
  }
  return records.map(record => JSON.stringify(record)).join('\n') + '\n';
}
try {
  await fs.mkdir(path.join(root, 'dist')); await fs.writeFile(path.join(root, 'dist/index.html'), '<html><head></head><body>API benchmark</body></html>');
  for (const provider of providers) {
    console.log(`${provider}: generating ${count} synthetic sessions`);
    const directory = path.join(root, provider), source = path.join(directory, 'source');
    const paths = [];
    for (let n = 0; n < count; n++) {
      const id = `s${String(n).padStart(6, '0')}`;
      const file = provider === 'grok' ? path.join(source, 'sessions/project', id, 'updates.jsonl') : path.join(source, provider === 'pi' ? 'sessions/project' : 'projects/project', `${id}.jsonl`);
      await fs.mkdir(path.dirname(file), { recursive: true }); await fs.writeFile(file, rows(provider, id)); paths.push(file);
      if (provider === 'grok') await fs.writeFile(path.join(path.dirname(file), 'summary.json'), JSON.stringify({ info: { id, cwd: '/benchmark' }, generated_title: id, created_at: '2026-09-30T00:00:00Z', updated_at: '2026-09-30T00:00:00Z', num_messages: messages, chat_format_version: 1 }));
    }
    const initial = await inventory(source);
    const settings = Object.fromEntries(['codex', 'claude', 'opencode', 'cursor', 'qoder', 'workbuddy', 'grok', 'pi', 'dsh', 'hermes', 'zcode', 'qwen', 'cline', 'copilot', 'antigravity'].map(name => [`${name}_dir`, '']));
    Object.assign(settings, { [`${provider}_dir`]: source, backup_dir: path.join(directory, 'backup'), open_command: 'auto', refresh_interval_ms: 5000, preview_only_messages: true, preview_collapse_process: true });
    const settingsFile = path.join(directory, 'settings.json'); await fs.writeFile(settingsFile, JSON.stringify(settings));
    const dirs = { codexDir: '', claudeDir: '', [`${provider}Dir`]: source };
    const result = { provider, source_bytes: initial.reduce((sum, file) => sum + file.bytes, 0), source_files: initial.length, phases: [] }; report.providers.push(result);
    let api = await start(directory, settingsFile); report.cli.version = await api('app_version');
    const began = performance.now(); const scan = await api('start_workbench_scan', { ...dirs, provider });
    const scanned = await poll(api, 'workbench_scan_status', scan.job_id);
    assert.equal(scanned.results.length, count);
    result.phases.push({ phase: 'scan', elapsed_ms: performance.now() - began, sessions: scanned.results.length });
    async function search(phase, query, indexed, reused, hits, expectedRole = 'assistant') {
      console.log(`${provider}: ${phase}`);
      const begin = performance.now();
      const job = await api('start_workbench_content_search', { ...dirs, query, scopes: [{ provider, rollout_paths: paths }] });
      const status = await poll(api, 'content_search_status', job.job_id);
      const elapsed = performance.now() - begin;
      assert.equal(status.indexed_files, indexed); assert.equal(status.reused_files, reused); assert.equal(status.results.length, hits); assert.equal(status.truncated, false); assert.equal(status.skipped_files, 0);
      if (hits) {
        assert.equal(path.resolve(status.results[0].session.rollout_path), path.resolve(paths[0]));
        const hit = status.results[0].matches.find(match => match.snippet.includes(query)); assert(hit); assert.equal(hit.role, expectedRole);
        const preview = await api('preview_session_range', { provider, rolloutPath: paths[0], offset: hit.event_offset, limit: 1 });
        assert.equal(preview.length, 1); assert.equal(preview[0].role, expectedRole); assert(preview[0].text_summary.includes(query));
      }
      result.phases.push({ phase, elapsed_ms: elapsed, indexed_files: status.indexed_files, reused_files: status.reused_files, matching_sessions: status.results.length, scanned_files: status.scanned_files, scanned_bytes: status.scanned_bytes });
    }
    await search('first_index_sparse', marker, count, 0, 1);
    for (let n = 1; n <= 3; n++) await search(`warm_sparse_${n}`, marker, 0, count, 1);
    await search('warm_absent', 'AVBENCH_ABSENT_QUERY', 0, count, 0);
    await search('warm_user_gate', userMarker, 0, count, 1, 'user');
    result.user_and_assistant_search_preview_verified = true;
    await stop(); api = await start(directory, settingsFile);
    const cacheStart = performance.now(); const cached = await api('cached_workbench_sessions', dirs); assert.equal(cached.sessions.length, count);
    result.phases.push({ phase: 'restart_cached_list', elapsed_ms: performance.now() - cacheStart, sessions: cached.sessions.length });
    await search('restart_sparse', marker, 0, count, 1);
    assert.deepEqual(await inventory(source), initial); result.source_hashes_unchanged_before_edit = true;
    const original = await fs.readFile(paths[0], 'utf8'); assert(original.includes(marker));
    await fs.writeFile(paths[0], original.replace(marker, changedMarker));
    const edited = await inventory(source);
    assert.equal(edited.filter((file, n) => file.sha256 !== initial[n].sha256).length, 1);
    await search('single_file_rebuild', changedMarker, 1, count - 1, 1);
    await search('old_marker_absent', marker, 0, count, 0);
    assert.deepEqual(await inventory(source), edited); result.source_hashes_unchanged_after_edit = true;
    await stop();
    result.index_files = await inventory(path.join(directory, 'search-index'));
    result.index_bytes = result.index_files.reduce((sum, file) => sum + file.bytes, 0);
    await fs.writeFile(path.join(directory, 'source-hashes.json'), JSON.stringify({ initial, after_controlled_edit: edited }, null, 2));
    await fs.writeFile(output, JSON.stringify(report, null, 2));
  }
  report.pass = true;
} catch (error) { report.error = String(error.stack ?? error); process.exitCode = 1; }
finally { await stop(); report.finished_at = new Date().toISOString(); await fs.writeFile(output, JSON.stringify(report, null, 2)); console.log(JSON.stringify({ pass: report.pass, output, fixture_root: root, error: report.error })); }
