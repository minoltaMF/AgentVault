use std::ffi::OsString;
use std::path::PathBuf;

use provider_codex::CodexProvider;
use provider_sdk::{
    ConsistencyClass, NativeSessionKind, NativeSessionRef, ProviderCapabilities, ResumeOptions,
    SessionProvider, SessionRoot, SessionRootKind,
};

#[test]
fn builds_prompt_free_native_resume_plan_by_session_id() {
    let provider = CodexProvider;
    let native = NativeSessionRef::new(
        "codex",
        Some("019d0000-1111-7000-8000-000000000001".into()),
        "/native/rollout.jsonl",
        SessionRoot::new(
            "/native",
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );

    let plan = provider
        .resume_plan(&native, &ResumeOptions::default())
        .expect("Codex resume plan");

    assert_eq!(plan.executable, PathBuf::from("codex"));
    assert_eq!(
        plan.args,
        vec![
            OsString::from("resume"),
            OsString::from("019d0000-1111-7000-8000-000000000001"),
        ]
    );
    assert!(provider.supports(ProviderCapabilities::NATIVE_RESUME));
    assert!(!provider.supports(ProviderCapabilities::MOVE_CWD));
    assert!(!provider.supports(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}
