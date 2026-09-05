use std::ffi::OsString;
use std::path::PathBuf;

use provider_claude::ClaudeProvider;
use provider_sdk::{
    ConsistencyClass, NativeSessionKind, NativeSessionRef, ProviderCapabilities, ResumeOptions,
    SessionProvider, SessionRoot, SessionRootKind,
};

#[test]
fn builds_prompt_free_native_resume_plan_by_session_id() {
    let provider = ClaudeProvider;
    let native = NativeSessionRef::new(
        "claude",
        Some("claude-session".into()),
        "/native/claude-session.jsonl",
        SessionRoot::new(
            "/native",
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );

    let plan = provider
        .resume_plan(&native, &ResumeOptions::default())
        .expect("Claude resume plan");

    assert_eq!(plan.executable, PathBuf::from("claude"));
    assert_eq!(
        plan.args,
        vec![OsString::from("--resume"), OsString::from("claude-session")]
    );
    assert!(provider.supports(ProviderCapabilities::NATIVE_RESUME));
    assert!(!provider.supports(ProviderCapabilities::MOVE_CWD));
    assert!(!provider.supports(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}

#[test]
fn does_not_claim_a_subagent_transcript_is_natively_resumable() {
    let native = NativeSessionRef::new(
        "claude",
        Some("agent-worker".into()),
        "/native/session/subagents/agent-worker.jsonl",
        SessionRoot::new(
            "/native",
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Subagent,
    );

    assert!(ClaudeProvider
        .resume_plan(&native, &ResumeOptions::default())
        .is_err());
}
