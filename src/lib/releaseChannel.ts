const compiledRepository = (
  import.meta as ImportMeta & { env?: Record<string, string | undefined> }
).env?.VITE_AGENTVAULT_RELEASE_REPOSITORY?.trim() ?? "";

export function configuredReleaseRepository(
  repository = compiledRepository,
): string {
  const normalized = repository.trim();
  const parts = normalized.split("/");
  const validComponent = (component: string) =>
    component.length > 0 &&
    component !== "." &&
    component !== ".." &&
    /^[A-Za-z0-9._-]+$/.test(component);

  if (parts.length !== 2 || !parts.every(validComponent)) {
    throw new Error(
      "AgentVault internal alpha 未配置独立更新源，请从原 internal alpha 分发渠道获取后续版本",
    );
  }
  return normalized;
}

export function latestReleaseApiUrl(repository = compiledRepository): string {
  return `https://api.github.com/repos/${configuredReleaseRepository(repository)}/releases/latest`;
}

export function releasesPageUrl(repository = compiledRepository): string {
  return `https://github.com/${configuredReleaseRepository(repository)}/releases`;
}
