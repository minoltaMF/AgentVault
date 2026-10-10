import { appendFileSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import path from 'node:path';

export function expectedAssets(version) {
  const names = [
    `AgentVault-${version}-1.x86_64.rpm`,
    `AgentVault_${version}_amd64.AppImage`,
    `AgentVault_${version}_amd64.deb`,
    `AgentVault_${version}_aarch64.dmg`,
    `AgentVault_${version}_x64.dmg`,
    'AgentVault_aarch64.app.tar.gz', 'AgentVault_x64.app.tar.gz',
    `AgentVault_${version}_x64-setup.exe`,
    ...['linux', 'windows', 'macos-arm64', 'macos-intel'].map(p => `cc-sessions-cli-v${version}-${p}.zip`),
    `cc-session-manager-portable-v${version}-windows.exe`,
    `cc-session-manager-portable-v${version}-windows.zip`,
  ];
  if (!version.includes('-')) names.push(`AgentVault_${version}_x64_en-US.msi`);
  return names;
}

export function desktopAssets(paths, platform, version) {
  const wanted = expectedAssets(version).filter(name => {
    if (name.startsWith('cc-')) return false;
    if (platform === 'linux') return /\.(rpm|deb|AppImage)$/.test(name);
    if (platform === 'windows') return /\.(exe|msi)$/.test(name);
    if (platform === 'macos-arm64') return name.includes('aarch64');
    if (platform === 'macos-intel') return /(_x64\.dmg|_x64\.app\.tar\.gz)$/.test(name);
    throw new Error('Invalid platform');
  });
  if (!Array.isArray(paths) || !paths.every(p => typeof p === 'string')) throw new Error('Invalid Tauri artifact paths');
  const files = paths.map(file => {
    // tauri-action emits artifactPaths before compressing the macOS .app directory.
    if (file.endsWith('.app')) file += '.tar.gz';
    let name = path.basename(file);
    if (name === 'AgentVault.app.tar.gz' && platform.startsWith('macos-')) {
      name = `AgentVault_${platform === 'macos-arm64' ? 'aarch64' : 'x64'}.app.tar.gz`;
    }
    return { file, name };
  });
  if (files.length !== wanted.length || new Set(files.map(f => f.name)).size !== wanted.length || files.some(f => !wanted.includes(f.name))) {
    throw new Error(`Desktop artifacts do not match ${platform}: ${files.map(f => f.name).join(', ')}`);
  }
  return files;
}

export function assertDraft(release, tag, sha) {
  if (!release.draft || release.tag_name !== tag || release.target_commitish !== sha || release.prerelease !== tag.includes('-')) {
    throw new Error('Release identity mismatch: expected an unpublished draft for this tag and commit');
  }
}

export function verifyAssets(assets, version) {
  const expected = new Set(expectedAssets(version));
  const seen = new Set();
  for (const asset of assets) {
    if (!expected.has(asset.name) || seen.has(asset.name)) throw new Error(`Unexpected or duplicate asset: ${asset.name}`);
    if (asset.state !== 'uploaded' || !Number.isSafeInteger(asset.size) || asset.size <= 0 || !/^sha256:[a-f0-9]{64}$/.test(asset.digest ?? '')) {
      throw new Error(`Incomplete asset: ${asset.name}`);
    }
    seen.add(asset.name);
  }
  const missing = [...expected].filter(name => !seen.has(name));
  if (missing.length) throw new Error(`Missing assets: ${missing.join(', ')}`);
  return seen.size;
}

// Injection keeps remote publication logic testable without credentials or network writes.
export function releaseService({ api, repo, tag, sha, version, wait = ms => new Promise(resolve => setTimeout(resolve, ms)) }) {
  const base = `/repos/${repo}`;
  if (tag !== `v${version}` || !/^[0-9a-f]{40}$/.test(sha) || !/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error('Invalid release context');
  async function list(route) {
    const all = [];
    for (let page = 1; ; page++) {
      const rows = await api(`${base}${route}?per_page=100&page=${page}`);
      if (!Array.isArray(rows)) throw new Error('Invalid paginated GitHub response');
      all.push(...rows);
      if (rows.length < 100) return all;
    }
  }
  async function assertTag() {
    let object = (await api(`${base}/git/ref/tags/${encodeURIComponent(tag)}`)).object;
    for (let depth = 0; object?.type === 'tag' && depth < 10; depth++) {
      object = (await api(`${base}/git/tags/${object.sha}`)).object;
    }
    if (object?.type !== 'commit' || object.sha !== sha) throw new Error('Tag does not resolve to the build commit');
  }
  async function unique() {
    const found = (await list('/releases')).filter(r => r.tag_name === tag);
    if (found.length > 1) throw new Error(`Multiple releases for ${tag}; refusing to choose or delete one`);
    return found[0];
  }
  async function checked(id) {
    if (!/^\d+$/.test(String(id))) throw new Error('Missing or invalid release ID');
    await assertTag();
    let release;
    // The list endpoint may briefly lag behind a successful create response.
    // Retry reads only; duplicate/foreign releases still fail immediately.
    for (const delay of [0, 1000, 2000, 4000, 8000, 16000]) {
      if (delay) await wait(delay);
      release = await unique();
      if (release) break;
    }
    if (!release || String(release.id) !== String(id)) throw new Error('Release ID is not the unique release for this tag');
    assertDraft(release, tag, sha);
    return release;
  }
  return {
    async prepare(body) {
      await assertTag();
      let release = await unique();
      if (!release) release = await api(`${base}/releases`, { method: 'POST', body: {
        tag_name: tag, target_commitish: sha, name: `AgentVault ${tag}`, body,
        draft: true, prerelease: tag.includes('-'),
      }});
      await checked(release.id);
      return release;
    },
    checked,
    async upload(id, name, data) {
      await checked(id);
      if (!expectedAssets(version).includes(name) || !data.length) throw new Error(`Invalid local asset: ${name}`);
      const digest = `sha256:${createHash('sha256').update(data).digest('hex')}`;
      const existing = (await list(`/releases/${id}/assets`)).filter(a => a.name === name);
      const valid = a => a.state === 'uploaded' && a.size === data.length && a.digest === digest;
      if (existing.length) {
        if (existing.length !== 1 || !valid(existing[0])) throw new Error(`Existing asset differs: ${name}; refusing to overwrite`);
        return existing[0];
      }
      let uploaded;
      try {
        uploaded = await api(`${base}/releases/${id}/assets?name=${encodeURIComponent(name)}`, { method: 'POST', binary: data, upload: true });
      } catch (error) {
        // A timed-out response can follow a successful upload. Never blindly repeat POST.
        await checked(id);
        const recovered = (await list(`/releases/${id}/assets`)).filter(a => a.name === name);
        if (recovered.length !== 1 || !valid(recovered[0])) throw error;
        uploaded = recovered[0];
      }
      if (!valid(uploaded)) throw new Error(`Uploaded asset checksum/size mismatch: ${name}`);
      return uploaded;
    },
    async verify(id) {
      const release = await checked(id);
      const assets = await list(`/releases/${id}/assets`);
      const count = verifyAssets(assets, version);
      for (const asset of assets) {
        const data = await api(`${base}/releases/assets/${asset.id}`, { download: true });
        if (data.length !== asset.size || `sha256:${createHash('sha256').update(data).digest('hex')}` !== asset.digest) {
          throw new Error(`Downloaded asset checksum/size mismatch: ${asset.name}`);
        }
      }
      await checked(id);
      return { release, count };
    },
  };
}

async function main() {
  const { GITHUB_REPOSITORY: repo, GITHUB_REF_NAME: tag, GITHUB_SHA: sha, GITHUB_TOKEN: token, RELEASE_ID: id } = process.env;
  if (!token) throw new Error('GITHUB_TOKEN is required');
  const version = JSON.parse(readFileSync('package.json', 'utf8')).version;
  const api = async (route, { method = 'GET', body, binary, upload = false, download = false } = {}) => {
    const response = await fetch(`https://${upload ? 'uploads' : 'api'}.github.com${route}`, {
      method, cache: 'no-store', signal: AbortSignal.timeout(120_000),
      headers: { Authorization: `Bearer ${token}`, Accept: download ? 'application/octet-stream' : 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28',
        'Content-Type': binary ? 'application/octet-stream' : 'application/json' },
      body: binary ?? (body ? JSON.stringify(body) : undefined),
    });
    if (!response.ok) throw new Error(`GitHub ${method} ${route}: HTTP ${response.status}`);
    return download ? Buffer.from(await response.arrayBuffer()) : response.json();
  };
  const service = releaseService({ api, repo, tag, sha, version });
  switch (process.argv[2]) {
    case 'prepare': {
      const release = await service.prepare(readFileSync(`docs/releases/${version}.md`, 'utf8'));
      appendFileSync(process.env.GITHUB_OUTPUT, `release_id=${release.id}\n`);
      console.log(`Prepared unique draft ${release.id}`);
      break;
    }
    case 'guard': await service.checked(id); break;
    case 'desktop': {
      const files = desktopAssets(JSON.parse(process.env.TAURI_ARTIFACT_PATHS ?? 'null'), process.env.RELEASE_PLATFORM, version);
      for (const { name, file } of files) {
        await service.upload(id, name, readFileSync(file));
        console.log(`Verified upload: ${name}`);
      }
      break;
    }
    case 'upload': {
      const platform = process.env.RELEASE_PLATFORM;
      if (!['linux', 'windows', 'macos-arm64', 'macos-intel'].includes(platform)) throw new Error('Invalid platform');
      const names = process.argv[3] === 'portable'
        ? (platform === 'windows' ? [`cc-session-manager-portable-v${version}-windows.exe`, `cc-session-manager-portable-v${version}-windows.zip`] : [])
        : process.argv[3] === 'cli' ? [`cc-sessions-cli-v${version}-${platform}.zip`] : [];
      if (!names.length) throw new Error('Invalid upload kind/platform');
      for (const name of names) {
        await service.upload(id, name, readFileSync(`release/${name}`));
        console.log(`Verified upload: ${name}`);
      }
      break;
    }
    case 'verify': {
      const { release, count } = await service.verify(id);
      const summary = `Verified ${count} uploaded, nonempty assets with downloaded SHA-256 verification in unique draft ${release.id}: ${release.html_url}\n`;
      console.log(summary);
      if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
      break;
    }
    default: throw new Error('Expected prepare, guard, upload or verify');
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
