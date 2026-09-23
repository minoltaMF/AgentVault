use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

pub use vault_io::path_safety::strip_verbatim;

pub fn basename_display(s: &str) -> String {
    let stripped = strip_verbatim(s);
    let p = Path::new(&stripped);
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| stripped.clone())
}

pub fn is_wsl_unc_path(path: &Path) -> bool {
    wsl_unc_mapping(path).is_some()
}

/// Map Linux absolute paths stored by Codex inside WSL back to the selected
/// Windows-accessible WSL UNC root. Non-WSL and non-Linux paths are unchanged.
pub fn host_path_from_codex_record(codex_dir: &Path, raw: &str) -> PathBuf {
    let cleaned = strip_verbatim(raw.trim());
    if cleaned.starts_with('/') {
        if let Some(mapping) = wsl_unc_mapping(codex_dir) {
            return mapping.host_path_for_linux_path(&cleaned);
        }
    }
    PathBuf::from(cleaned)
}

pub fn host_path_string_from_codex_record(codex_dir: &Path, raw: &str) -> String {
    host_path_from_codex_record(codex_dir, raw)
        .to_string_lossy()
        .into_owned()
}

/// Convert a host-visible project path back to the path format stored by Codex.
///
/// A Codex directory selected through WSL is exposed to the Windows desktop app as a UNC path,
/// while the Codex process inside WSL expects Linux paths in `session_meta.cwd` and `threads.cwd`.
/// Refuse paths outside the selected distro instead of persisting an unusable Windows path.
pub fn codex_record_path_from_host(codex_dir: &Path, host_path: &Path) -> AppResult<String> {
    let cleaned = strip_verbatim(&host_path.to_string_lossy());
    if let Some(mapping) = wsl_unc_mapping(codex_dir) {
        return mapping.linux_path_for_host_path(&cleaned).ok_or_else(|| {
            AppError::Path(format!(
                "WSL Codex 的项目目录必须位于当前发行版 {} 中: {}",
                mapping.unc_root, cleaned
            ))
        });
    }
    Ok(cleaned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WslUncMapping {
    unc_root: String,
    distro: String,
}

impl WslUncMapping {
    fn host_path_for_linux_path(&self, linux_path: &str) -> PathBuf {
        let mut out = self.unc_root.clone();
        for segment in linux_path.trim_start_matches('/').split('/') {
            if !segment.is_empty() {
                out.push('\\');
                out.push_str(segment);
            }
        }
        PathBuf::from(out)
    }

    fn linux_path_for_host_path(&self, host_path: &str) -> Option<String> {
        let (_, distro, segments) = parse_wsl_unc(host_path)?;
        if !distro.eq_ignore_ascii_case(&self.distro) {
            return None;
        }
        if segments.is_empty() {
            return Some("/".to_string());
        }
        Some(format!("/{}", segments.join("/")))
    }
}

fn parse_wsl_unc(raw: &str) -> Option<(String, String, Vec<String>)> {
    let normalized = strip_verbatim(raw).replace('/', "\\");
    let rest = normalized.strip_prefix(r"\\")?;
    let mut parts = rest.split('\\').filter(|part| !part.is_empty());
    let server = parts.next()?.to_string();
    if !server.eq_ignore_ascii_case("wsl.localhost") && !server.eq_ignore_ascii_case("wsl$") {
        return None;
    }
    let distro = parts.next()?.to_string();
    let segments = parts.map(str::to_string).collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| segment == "." || segment == "..")
    {
        return None;
    }
    Some((server, distro, segments))
}

fn wsl_unc_mapping(path: &Path) -> Option<WslUncMapping> {
    let (server, distro, _) = parse_wsl_unc(&path.to_string_lossy())?;
    Some(WslUncMapping {
        unc_root: format!(r"\\{}\{}", server, distro),
        distro,
    })
}

pub fn default_codex_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

pub fn default_qoder_dir() -> PathBuf {
    std::env::var_os("QODER_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".qoder"))
}
pub fn default_claude_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".claude"))
        .unwrap_or_else(|| PathBuf::from(".claude"))
}

pub fn default_opencode_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".local").join("share").join("opencode"))
        .unwrap_or_else(|| PathBuf::from(".local/share/opencode"))
}

/// Cursor IDE 的用户数据目录，即 VS Code 系的 `<config>/Cursor/User`。
///
/// `dirs::config_dir()` 在三个平台上恰好给出 Cursor 使用的父目录：
/// macOS `~/Library/Application Support`、Windows `%APPDATA%`、Linux `~/.config`。
/// 会话主体存放在其下的 `globalStorage/state.vscdb`。
pub fn default_cursor_dir() -> PathBuf {
    dirs::config_dir()
        .map(|config| config.join("Cursor").join("User"))
        .unwrap_or_else(|| PathBuf::from("Cursor/User"))
}

/// cursor-agent CLI 的家目录 `~/.cursor`，与 IDE 数据目录相互独立。
///
/// 只有 `chats/` 子目录里的会话对本工具有意义；`projects/*/agent-transcripts/`
/// 是同一批会话的有损展示副本（无工具结果、无时间戳），刻意不读。
pub fn default_cursor_agent_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".cursor"))
        .unwrap_or_else(|| PathBuf::from(".cursor"))
}

/// Cursor IDE 的会话数据库。
pub fn cursor_state_db_path(cursor_dir: &Path) -> PathBuf {
    cursor_dir.join("globalStorage").join("state.vscdb")
}

/// VS Code 系的工作区元数据目录，用于把 `workspaceId` 回查成项目路径。
pub fn cursor_workspace_storage_dir(cursor_dir: &Path) -> PathBuf {
    cursor_dir.join("workspaceStorage")
}

/// cursor-agent 的会话根目录，布局为 `chats/<md5(cwd)>/<agentId>/`。
pub fn cursor_agent_chats_dir(agent_dir: &Path) -> PathBuf {
    agent_dir.join("chats")
}

pub fn default_backup_dir() -> PathBuf {
    let cc_root = default_codex_dir();
    cc_root
        .parent()
        .map(|p| p.join("cc-backups"))
        .unwrap_or_else(|| PathBuf::from("cc-backups"))
}

pub fn validate_codex_dir(path: &Path) -> (bool, bool, bool) {
    let exists = path.is_dir();
    let has_state = path.join("state_5.sqlite").is_file();
    let has_sessions = path.join("sessions").is_dir();
    (exists, has_state, has_sessions)
}

pub fn validate_claude_dir(path: &Path) -> (bool, bool) {
    let exists = path.is_dir();
    let has_projects = path.join("projects").is_dir();
    (exists, has_projects)
}

pub fn claude_projects_dir(claude: &Path) -> PathBuf {
    claude.join("projects")
}

/// 所有与 Codex 目录相关的关键子路径集中在此，方便其他模块引用。
pub fn sessions_dir(codex: &Path) -> PathBuf {
    codex.join("sessions")
}

pub fn archived_sessions_dir(codex: &Path) -> PathBuf {
    codex.join("archived_sessions")
}

pub fn session_index_path(codex: &Path) -> PathBuf {
    codex.join("session_index.jsonl")
}

pub fn history_path(codex: &Path) -> PathBuf {
    codex.join("history.jsonl")
}

pub fn state_db_path(codex: &Path) -> PathBuf {
    codex.join("state_5.sqlite")
}

pub fn config_toml_path(codex: &Path) -> PathBuf {
    codex.join("config.toml")
}

/// Codex App 的 Electron 全局状态文件：维护当前的本地项目定义、会话项目归属和
/// project-order；写回时保留应用拥有的其他未知字段。只更新 rollout/SQLite cwd 时，
/// 官方 App 的左侧项目列表不会把会话移动到目标项目。
pub fn codex_global_state_json_path(codex: &Path) -> PathBuf {
    codex.join(".codex-global-state.json")
}

/// manager 自己维护的家族树元数据文件（Codex 原生不感知）。
pub fn family_store_path(codex: &Path) -> PathBuf {
    codex.join("session_family.json")
}

/// CC Sessions 自己维护的归档来源登记，Codex/Claude 原生均不读取。
/// 放在 codex_home 根目录（与 session_family.json 平级），不放 sessions/ 或
/// archived_sessions/——官方 `codex doctor` 的 scan_rollout_files 会扫描这两个根。
pub fn archive_ledger_path(codex: &Path) -> PathBuf {
    codex.join("archive_ledger.json")
}

/// CC Sessions 自己维护的转换来源登记，Codex/Claude 原生均不读取。
pub fn session_provenance_path(codex: &Path) -> PathBuf {
    codex.join("session_provenance.json")
}

/// 从 rollout 绝对路径推算相对于 codex_dir 的相对路径。
/// 若不是 codex 子路径则返回 `sessions/<basename>`（保底）。
#[allow(dead_code)]
pub fn rollout_relpath(abs: &str, codex: &Path) -> PathBuf {
    let abs_clean = strip_verbatim(abs);
    let codex_clean = strip_verbatim(&codex.to_string_lossy());
    let abs_p = PathBuf::from(&abs_clean);
    let cx_p = PathBuf::from(&codex_clean);
    match abs_p.strip_prefix(&cx_p) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => abs_p
            .file_name()
            .map(|n| PathBuf::from("sessions").join(n))
            .unwrap_or_else(|| PathBuf::from("sessions/unknown.jsonl")),
    }
}

/// 机器标识：优先取环境变量 `CSM_MACHINE_LABEL`，否则用 hostname/COMPUTERNAME。
pub fn machine_label() -> String {
    if let Ok(v) = std::env::var("CSM_MACHINE_LABEL") {
        if !v.trim().is_empty() {
            return sanitize_slug(v.trim());
        }
    }
    if let Ok(v) = std::env::var("COMPUTERNAME") {
        if !v.trim().is_empty() {
            return sanitize_slug(v.trim());
        }
    }
    if let Ok(v) = std::env::var("HOSTNAME") {
        if !v.trim().is_empty() {
            return sanitize_slug(v.trim());
        }
    }
    "unknown-machine".into()
}

/// 把任意字符串变成跨平台安全的文件/目录名片段。
pub fn sanitize_slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
        out.push(if ok { c } else { '_' });
    }
    if out.is_empty() {
        "_".into()
    } else {
        out
    }
}

/// 校验外部 manifest / zip 中声明的相对路径，拒绝绝对路径和目录穿越。
pub fn checked_relative_path(raw: &str) -> AppResult<PathBuf> {
    vault_io::path_safety::checked_relative_path(raw).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_linux_paths_under_wsl_unc_root() {
        let codex = Path::new(r"\\wsl.localhost\Ubuntu\home\alice\.codex");
        let mapped =
            host_path_string_from_codex_record(codex, "/home/alice/.codex/sessions/a.jsonl");

        assert_eq!(
            mapped,
            r"\\wsl.localhost\Ubuntu\home\alice\.codex\sessions\a.jsonl"
        );
    }

    #[test]
    fn maps_linux_paths_under_wsl_dollar_unc_root() {
        let codex = Path::new(r"\\wsl$\Ubuntu\home\alice\.codex");
        let mapped = host_path_string_from_codex_record(codex, "/home/alice/project");

        assert_eq!(mapped, r"\\wsl$\Ubuntu\home\alice\project");
    }

    #[test]
    fn leaves_non_wsl_linux_paths_unchanged() {
        let codex = Path::new(r"C:\Users\alice\.codex");
        let mapped = host_path_string_from_codex_record(codex, "/home/alice/.codex/a.jsonl");

        assert_eq!(mapped, r"/home/alice/.codex/a.jsonl");
    }

    #[test]
    fn maps_wsl_host_paths_back_to_linux_records() -> AppResult<()> {
        let codex = Path::new(r"\\wsl.localhost\Ubuntu\home\alice\.codex");
        let host = Path::new(r"\\wsl.localhost\Ubuntu\home\alice\project");

        assert_eq!(
            codex_record_path_from_host(codex, host)?,
            "/home/alice/project"
        );
        Ok(())
    }

    #[test]
    fn maps_wsl_host_paths_across_unc_aliases_and_ascii_case() -> AppResult<()> {
        let codex = Path::new(r"\\wsl$\Ubuntu\home\alice\.codex");
        let host = Path::new(r"\\WSL.LOCALHOST\ubuntu\home\alice\project");

        assert_eq!(
            codex_record_path_from_host(codex, host)?,
            "/home/alice/project"
        );
        Ok(())
    }

    #[test]
    fn rejects_host_paths_outside_the_selected_wsl_distro() {
        let codex = Path::new(r"\\wsl$\Ubuntu\home\alice\.codex");
        let host = Path::new(r"C:\work\project");

        assert!(codex_record_path_from_host(codex, host).is_err());

        let other_distro = Path::new(r"\\wsl.localhost\Ubuntu-Preview\home\alice\project");
        assert!(codex_record_path_from_host(codex, other_distro).is_err());

        let unicode_distro = Path::new(r"\\wsl.localhost\发行版\home\alice\project");
        assert!(codex_record_path_from_host(codex, unicode_distro).is_err());

        let traversal = Path::new(r"\\wsl.localhost\Ubuntu\home\alice\..\bob\project");
        assert!(codex_record_path_from_host(codex, traversal).is_err());
    }
}

pub fn default_workbuddy_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".workbuddy")
}
pub fn default_grok_dir() -> PathBuf {
    std::env::var_os("GROK_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".grok"))
}
pub fn default_pi_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    match std::env::var_os("PI_CODING_AGENT_DIR").filter(|v| !v.is_empty()) {
        Some(value) => {
            let value = value.to_string_lossy();
            if value == "~" {
                home
            } else if let Some(rest) = value
                .strip_prefix("~/")
                .or_else(|| value.strip_prefix("~\\"))
            {
                home.join(rest)
            } else {
                PathBuf::from(value.as_ref())
            }
        }
        None => home.join(".pi").join("agent"),
    }
}

pub fn default_hermes_dir() -> PathBuf {
    std::env::var_os("HERMES_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".hermes"))
}
pub fn default_zcode_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".zcode")
}
pub fn default_dsh_dir() -> PathBuf {
    std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".dsh"))
}
