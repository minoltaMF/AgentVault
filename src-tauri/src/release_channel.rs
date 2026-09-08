use crate::error::{AppError, AppResult};

const RELEASE_REPOSITORY: Option<&str> = option_env!("AGENTVAULT_RELEASE_REPOSITORY");
const NOT_CONFIGURED_MESSAGE: &str =
    "AgentVault internal alpha 未配置独立更新源，请从原 internal alpha 分发渠道获取后续版本";

#[cfg(feature = "desktop")]
pub(crate) fn latest_release_api_url() -> AppResult<String> {
    Ok(format!(
        "https://api.github.com/repos/{}/releases/latest",
        configured_repository()?
    ))
}

pub(crate) fn releases_page_url() -> AppResult<String> {
    Ok(format!(
        "https://github.com/{}/releases",
        configured_repository()?
    ))
}

fn configured_repository() -> AppResult<&'static str> {
    let repository = RELEASE_REPOSITORY
        .map(str::trim)
        .filter(|repository| !repository.is_empty())
        .ok_or_else(|| AppError::Other(NOT_CONFIGURED_MESSAGE.to_string()))?;
    validate_repository(repository)?;
    Ok(repository)
}

fn validate_repository(repository: &str) -> AppResult<()> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    let valid_component = |component: &str| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    if parts.next().is_some() || !valid_component(owner) || !valid_component(name) {
        return Err(AppError::Other(
            "AGENTVAULT_RELEASE_REPOSITORY 必须使用 owner/repository 格式".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_bounded_github_repository_name() {
        assert!(validate_repository("agentvault/agentvault").is_ok());
        assert!(validate_repository("owner-name/repo.name").is_ok());
    }

    #[test]
    fn rejects_missing_or_unsafe_repository_components() {
        for repository in [
            "",
            "owner",
            "/repo",
            "owner/",
            "owner/repo/extra",
            "../repo",
            "owner/..",
            "owner/repo?tab=tags",
        ] {
            assert!(
                validate_repository(repository).is_err(),
                "unexpected valid repository: {repository}"
            );
        }
    }
}
