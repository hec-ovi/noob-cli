//! Lazy `.env` semantics: keys are resolved fresh on every request build.
//! The full named `hot_reload_env` e2e (through the compiled binary) lands in
//! P1; this covers the provider-layer half in P0.

use noob_provider::resolve_endpoint;
use noob_provider::types::{ApiStyle, Overrides, ProviderError};

#[test]
fn env_edits_apply_on_next_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let env_path = dir.path().join(".env");

    std::fs::write(
        &env_path,
        "NOOB_BASE_URL=http://one:1/v1\nNOOB_API_KEY=k1\n",
    )
    .unwrap();
    let ep1 = resolve_endpoint(dir.path(), &Overrides::default()).unwrap();
    assert_eq!(ep1.base_url, "http://one:1/v1");
    assert_eq!(ep1.api_key, "k1");

    std::fs::write(
        &env_path,
        "NOOB_BASE_URL=http://two:2/v1\nNOOB_API_KEY=k2\n",
    )
    .unwrap();
    let ep2 = resolve_endpoint(dir.path(), &Overrides::default()).unwrap();
    assert_eq!(ep2.base_url, "http://two:2/v1");
    assert_eq!(ep2.api_key, "k2");
}

#[test]
fn missing_base_url_names_the_file_and_the_fix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "# empty\n").unwrap();
    let err = resolve_endpoint(dir.path(), &Overrides::default()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("NOOB_BASE_URL"), "{msg}");
    assert!(msg.contains(".env"), "{msg}");
}

#[test]
fn api_style_defaults_by_host_and_accepts_override() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".env"),
        "NOOB_BASE_URL=https://api.openai.com/v1\n",
    )
    .unwrap();
    let ep = resolve_endpoint(dir.path(), &Overrides::default()).unwrap();
    assert_eq!(ep.style, ApiStyle::Responses);

    std::fs::write(
        dir.path().join(".env"),
        "NOOB_BASE_URL=https://api.openai.com/v1\nNOOB_API_STYLE=chat\n",
    )
    .unwrap();
    let ep = resolve_endpoint(dir.path(), &Overrides::default()).unwrap();
    assert_eq!(ep.style, ApiStyle::Chat);

    std::fs::write(
        dir.path().join(".env"),
        "NOOB_BASE_URL=http://localhost:8090/v1\n",
    )
    .unwrap();
    let ep = resolve_endpoint(dir.path(), &Overrides::default()).unwrap();
    assert_eq!(ep.style, ApiStyle::Chat);
}

#[test]
fn bad_api_style_states_the_valid_values() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".env"),
        "NOOB_BASE_URL=http://x:1/v1\nNOOB_API_STYLE=grpc\n",
    )
    .unwrap();
    let err = resolve_endpoint(dir.path(), &Overrides::default()).unwrap_err();
    assert!(matches!(err, ProviderError::Config(_)));
    assert!(err.to_string().contains("chat"), "{err}");
}

#[test]
fn reasoning_is_unset_by_default_and_reads_both_spellings() {
    let dir = tempfile::tempdir().unwrap();
    let write = |extra: &str| {
        std::fs::write(
            dir.path().join(".env"),
            format!("NOOB_BASE_URL=http://x:1/v1\n{extra}"),
        )
        .unwrap();
    };

    write("");
    assert_eq!(
        resolve_endpoint(dir.path(), &Overrides::default())
            .unwrap()
            .reasoning,
        None
    );

    for off in ["off", "OFF", "false", "no", "0", " off "] {
        write(&format!("NOOB_REASONING={off}\n"));
        assert_eq!(
            resolve_endpoint(dir.path(), &Overrides::default())
                .unwrap()
                .reasoning,
            Some(false),
            "{off:?}"
        );
    }

    for on in ["on", "true", "yes", "1"] {
        write(&format!("NOOB_REASONING={on}\n"));
        assert_eq!(
            resolve_endpoint(dir.path(), &Overrides::default())
                .unwrap()
                .reasoning,
            Some(true),
            "{on:?}"
        );
    }
}

#[test]
fn bad_reasoning_value_states_the_valid_values() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".env"),
        "NOOB_BASE_URL=http://x:1/v1\nNOOB_REASONING=maybe\n",
    )
    .unwrap();
    let err = resolve_endpoint(dir.path(), &Overrides::default()).unwrap_err();
    assert!(matches!(err, ProviderError::Config(_)));
    let msg = err.to_string();
    assert!(msg.contains("on") && msg.contains("off"), "{msg}");
}

#[test]
fn trailing_slash_on_base_url_is_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "NOOB_BASE_URL=http://x:1/v1/\n").unwrap();
    let ep = resolve_endpoint(dir.path(), &Overrides::default()).unwrap();
    assert_eq!(ep.base_url, "http://x:1/v1");
}
