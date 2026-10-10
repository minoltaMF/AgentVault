import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { expectedAssets, desktopAssets, verifyAssets, releaseService } from './release-draft.mjs';
const version = '0.1.0-alpha.21', tag = `v${version}`, sha = 'a'.repeat(40);
const data = Buffer.from('synthetic artifact');
const digest = `sha256:${createHash('sha256').update(data).digest('hex')}`;
const draft = () => ({ id: 7, tag_name: tag, target_commitish: sha, draft: true, prerelease: true });
const asset = (name = expectedAssets(version)[0], id = 1) => ({ id, name, size: data.length, state: 'uploaded', digest });
function mock(options = {}) {
  let releases = options.releases ?? [], assets = options.assets ?? [];
  const calls = [];
  const api = async (route, input = {}) => {
    calls.push({ route, ...input });
    if (route.includes('/git/ref/')) return { object: { type: 'tag', sha: 'b'.repeat(40) } };
    if (route.includes('/git/tags/')) return { object: { type: 'commit', sha: options.sha ?? sha } };
    if (route.includes('/releases?')) {
      if (options.pagination && route.endsWith('page=1')) return Array.from({ length: 100 }, (_, i) => ({ tag_name: `other-${i}` }));
      return releases;
    }
    if (route.endsWith('/releases') && input.method === 'POST') {
      const created = { id: 7, ...input.body }; releases.push(created); return created;
    }
    if (route.includes('/assets?') && input.method === 'POST') {
      const uploaded = asset(new URL(`https://example.test${route}`).searchParams.get('name'));
      assets.push(uploaded);
      if (options.timeout) throw new Error('response timed out');
      return uploaded;
    }
    if (route.includes('/assets?')) return assets;
    if (input.download) return options.corrupt ? Buffer.from('bad') : data;
    throw new Error(`Unexpected mock request ${route}`);
  };
  return { calls, service: releaseService({ api, repo: 'owner/repo', version, tag, sha }) };
}
test('create once and reuse the same draft on repeated preparation', async () => {
  const { service, calls } = mock();
  assert.equal((await service.prepare('notes')).id, 7);
  assert.equal((await service.prepare('notes')).id, 7);
  assert.equal(calls.filter(c => c.method === 'POST').length, 1);
  assert.equal(calls.find(c => c.method === 'POST').body.draft, true);
});
test('find existing draft beyond the first page', async () => {
  const { service, calls } = mock({ releases: [draft()], pagination: true });
  assert.equal((await service.prepare('notes')).id, 7);
  assert.ok(calls.some(c => c.route.endsWith('page=2')));
  assert.ok(!calls.some(c => c.method === 'POST'));
});
for (const [name, releases, wrongSha] of [
  ['duplicate drafts', [draft(), { ...draft(), id: 8 }]],
  ['published release', [{ ...draft(), draft: false }]],
  ['wrong commit identity', [{ ...draft(), target_commitish: 'c'.repeat(40) }]],
  ['wrong prerelease state', [{ ...draft(), prerelease: false }]],
  ['moved annotated tag', [draft()], 'c'.repeat(40)],
]) test(`reject ${name} without remote writes`, async () => {
  const { service, calls } = mock({ releases, sha: wrongSha, pagination: true });
  await assert.rejects(service.prepare('notes'));
  assert.ok(!calls.some(c => c.method));
});
test('refuse another release ID and a draft published after preparation', async () => {
  const releases = [draft()]; const { service, calls } = mock({ releases });
  await assert.rejects(service.checked(8));
  await service.prepare('notes'); releases[0].draft = false;
  await assert.rejects(service.upload(7, asset().name, data));
  assert.ok(!calls.some(c => c.method === 'POST'));
});
test('upload only by ID, reuse identical bytes, recover a lost successful response', async () => {
  const { service, calls } = mock({ releases: [draft()], timeout: true });
  await service.upload(7, asset().name, data);
  await service.upload(7, asset().name, data);
  const posts = calls.filter(c => c.method === 'POST');
  assert.equal(posts.length, 1);
  assert.match(posts[0].route, /\/releases\/7\/assets\?name=/);
});
test('conflicting existing asset is never overwritten', async () => {
  const { service, calls } = mock({ releases: [draft()], assets: [{ ...asset(), digest: `sha256:${'0'.repeat(64)}` }] });
  await assert.rejects(service.upload(7, asset().name, data), /refusing to overwrite/);
  assert.ok(!calls.some(c => c.method));
});
test('verify exact alpha and stable platform manifest', () => {
  assert.equal(verifyAssets(expectedAssets(version).map(asset), version), 14);
  assert.equal(verifyAssets(expectedAssets('0.1.0').map(asset), '0.1.0'), 15);
});
for (const [name, mutate] of [
  ['missing file', a => a.slice(1)], ['extra file', a => [...a, asset('unknown.zip')]],
  ['duplicate filename', a => [...a, a[0]]], ['empty file', a => [{ ...a[0], size: 0 }, ...a.slice(1)]],
  ['unfinished upload', a => [{ ...a[0], state: 'starter' }, ...a.slice(1)]],
  ['missing digest', a => [{ ...a[0], digest: null }, ...a.slice(1)]],
  ['malformed digest', a => [{ ...a[0], digest: 'sha256:wrong' }, ...a.slice(1)]],
]) test(`integrity gate rejects ${name}`, () => {
  assert.throws(() => verifyAssets(mutate(expectedAssets(version).map(asset)), version));
});
test('download each asset and reject content corruption', async () => {
  const options = { releases: [draft()], assets: expectedAssets(version).map(asset) };
  const good = mock(options);
  assert.equal((await good.service.verify(7)).count, 14);
  assert.equal(good.calls.filter(c => c.download).length, 14);
  await assert.rejects(mock({ ...options, corrupt: true }).service.verify(7), /checksum/);
});
test('desktop paths keep architecture names and exclude unexpected artifacts', () => {
  for (const platform of ['macos-arm64', 'macos-intel']) {
    const arch = platform === 'macos-arm64' ? 'aarch64' : 'x64';
    const files = desktopAssets([`/bundle/AgentVault_${version}_${arch}.dmg`, '/bundle/AgentVault.app'], platform, version);
    assert.equal(files[1].name, `AgentVault_${arch}.app.tar.gz`);
    assert.equal(files[1].file, '/bundle/AgentVault.app.tar.gz');
  }
  assert.equal(desktopAssets([`/bundle/AgentVault_${version}_x64-setup.exe`], 'windows', version).length, 1);
  assert.throws(() => desktopAssets([], 'linux', version));
  assert.throws(() => desktopAssets(['/bundle/other.exe'], 'windows', version));
});
test('workflow has one preparation, common release ID, no tag uploads, and mandatory final gate', () => {
  const workflow = readFileSync('.github/workflows/release.yml', 'utf8');
  assert.match(workflow, /cancel-in-progress: false/);
  assert.match(workflow, /needs: prepare/);
  assert.equal((workflow.match(/release-draft\.mjs prepare/g) ?? []).length, 1);
  assert.match(workflow, /RELEASE_ID: \$\{\{ needs.prepare.outputs.release_id \}\}/);
  assert.match(workflow, /needs: \[prepare, build\]/);
  assert.match(workflow, /if: \$\{\{ always\(\) \}\}/);
  assert.match(workflow, /release-draft\.mjs verify/);
  assert.doesNotMatch(workflow, /gh release|--clobber|^\s+(tagName|releaseId|releaseName):/m);
});
