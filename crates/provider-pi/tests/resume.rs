use std::ffi::OsString;
use std::path::PathBuf;

use provider_pi::PiProvider;
use provider_sdk::{
    ConsistencyClass, NativeSessionKind, NativeSessionRef, ProviderCapabilities, ResumeOptions,
    SessionProvider, SessionRoot, SessionRootKind,
};

#[test]
fn builds_prompt_free_native_resume_plan_by_session_id() {
    let provider = PiProvider;
    let native = NativeSessionRef::new(
        "pi",
        Some("pi-session".into()),
        "/native/pi-session.jsonl",
        SessionRoot::new(
            "/native",
            SessionRootKind::Active,
            ConsistencyClass::AppendOnlyJsonl,
        ),
        NativeSessionKind::Primary,
    );

    let plan = provider
        .resume_plan(&native, &ResumeOptions::default())
        .expect("Pi resume plan");

    assert_eq!(plan.executable, PathBuf::from("pi"));
    assert_eq!(
        plan.args,
        vec![OsString::from("--session"), OsString::from("pi-session")]
    );
    assert!(provider.supports(ProviderCapabilities::NATIVE_RESUME));
    assert!(!provider.supports(ProviderCapabilities::MOVE_CWD));
    assert!(!provider.supports(ProviderCapabilities::WRITE_NATIVE_UNSAFE));
}
