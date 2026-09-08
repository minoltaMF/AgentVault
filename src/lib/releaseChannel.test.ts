import assert from "node:assert/strict";
import test from "node:test";
import {
  configuredReleaseRepository,
  latestReleaseApiUrl,
  releasesPageUrl,
} from "./releaseChannel.ts";

test("builds release URLs only from a bounded repository name", () => {
  assert.equal(
    latestReleaseApiUrl("agentvault/agentvault"),
    "https://api.github.com/repos/agentvault/agentvault/releases/latest",
  );
  assert.equal(
    releasesPageUrl("owner-name/repo.name"),
    "https://github.com/owner-name/repo.name/releases",
  );
});

test("rejects an absent or unsafe release repository", () => {
  assert.throws(() => latestReleaseApiUrl());
  assert.throws(() => releasesPageUrl());
  for (const repository of [
    "",
    "owner",
    "/repo",
    "owner/",
    "owner/repo/extra",
    "../repo",
    "owner/..",
    "owner/repo?tab=tags",
  ]) {
    assert.throws(() => configuredReleaseRepository(repository));
  }
});
