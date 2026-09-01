// Focused coverage: user-defined [providers.<name>] config profiles must
// appear in the /login and /logout pickers, resolve via /login <name> after
// statics, store keys in ~/.config/jcode/<profile>.env, and clear on logout.

fn wafer_config_toml() -> &'static str {
    r#"
[providers.wafer]
type = "anthropic-compatible"
base_url = "https://pass.wafer.ai"
auth = "header"
api_key_env = "WAFER_TEST_KEY_ENV"
default_model = "GLM-5.2"

[[providers.wafer.models]]
id = "GLM-5.2"

[[providers.wafer.models]]
id = "Qwen3.5-397B-A17B"
"#
}

struct ProfileEnvGuard {
    previous_home: Option<std::ffi::OsString>,
    key_envs: &'static [&'static str],
    previous_keys: Vec<Option<std::ffi::OsString>>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl ProfileEnvGuard {
    fn new(key_envs: &'static [&'static str]) -> Self {
        let _lock = crate::storage::lock_test_env();
        let previous_home = std::env::var_os("JCODE_HOME");
        let temp = tempfile::TempDir::new().expect("tempdir");
        crate::env::set_var("JCODE_HOME", temp.path());
        let config_path = crate::config::Config::path().expect("config path");
        std::fs::create_dir_all(config_path.parent().expect("config parent")).expect("mkdir");
        std::fs::write(&config_path, wafer_config_toml()).expect("write config");
        crate::config::Config::invalidate_cache();
        let previous_keys = key_envs
            .iter()
            .map(|name| std::env::var_os(name))
            .collect::<Vec<_>>();
        for name in key_envs {
            crate::env::remove_var(name);
        }
        std::mem::forget(temp); // dropped in Drop to keep JCODE_HOME valid
        Self {
            previous_home,
            key_envs,
            previous_keys,
            _lock,
        }
    }
}

impl Drop for ProfileEnvGuard {
    fn drop(&mut self) {
        crate::config::Config::invalidate_cache();
        if let Some(home) = &self.previous_home {
            crate::env::set_var("JCODE_HOME", home);
        } else {
            crate::env::remove_var("JCODE_HOME");
        }
        for (name, previous) in self.key_envs.iter().zip(self.previous_keys.drain(..)) {
            match previous {
                Some(value) => crate::env::set_var(name, value),
                None => crate::env::remove_var(name),
            }
        }
        crate::config::Config::invalidate_cache();
    }
}

#[test]
fn named_profile_login_appears_in_login_and_logout_pickers() {
    let _guard = ProfileEnvGuard::new(&["WAFER_TEST_KEY_ENV"]);
    let mut app = create_test_app();

    app.open_login_picker_inline();
    let entries = app.inline_interactive_state.as_ref().expect("login picker");
    assert!(
        entries
            .entries
            .iter()
            .any(|entry| entry.name == "wafer (config profile)"
                && matches!(entry.action, crate::tui::PickerAction::LoginProfile(_))),
        "login picker must list config profiles"
    );

    app.inline_interactive_state = None;
    app.open_logout_picker_inline();
    let entries = app
        .inline_interactive_state
        .as_ref()
        .expect("logout picker");
    assert!(
        entries
            .entries
            .iter()
            .any(|entry| entry.name == "wafer (config profile)"
                && matches!(entry.action, crate::tui::PickerAction::LogoutProfile(_))),
        "logout picker must list config profiles"
    );
}

#[test]
fn named_profile_selection_resolves_name_after_statics() {
    let _guard = ProfileEnvGuard::new(&["WAFER_TEST_KEY_ENV"]);
    let providers = crate::provider_catalog::tui_login_providers();

    // Static provider still resolves through the static path.
    assert!(
        crate::provider_catalog::resolve_login_selection("openrouter", &providers).is_some(),
        "statics must keep resolving"
    );

    // Profile name resolves via the fallback.
    let profile = crate::tui::app::auth::resolve_named_profile_selection("wafer", &providers)
        .expect("wafer profile must resolve");
    assert_eq!(profile.api_key_env, "WAFER_TEST_KEY_ENV");
    assert_eq!(profile.env_file, "wafer.env");
    assert_eq!(profile.base_url, "https://pass.wafer.ai");

    // Unknown names fail.
    assert!(crate::tui::app::auth::resolve_named_profile_selection("not-a-provider", &providers).is_none());
}

#[test]
fn named_profile_login_saves_key_and_logout_clears_it() {
    let _guard = ProfileEnvGuard::new(&["WAFER_TEST_KEY_ENV"]);
    let mut app = create_test_app();

    app.start_named_profile_login(crate::tui::app::auth::resolve_named_profile_selection("wafer", &[]).expect("profile"));
    assert!(
        app.pending_login.is_some(),
        "login must stage a pending key prompt"
    );

    // Submit a key through the shared pending-login path.
    let pending = app.pending_login.take().expect("pending");
    app.handle_login_input(pending, "waf-test-key-1234567890".to_string());

    let env_path = crate::storage::app_config_dir()
        .expect("config dir")
        .join("wafer.env");
    let saved = std::fs::read_to_string(&env_path).expect("wafer.env written");
    assert!(
        saved.contains("WAFER_TEST_KEY_ENV=waf-test-key-1234567890"),
        "key must be persisted to the profile env file, got: {saved}"
    );

    // Logout clears the stored key.
    app.start_named_profile_logout(
        crate::tui::app::auth::resolve_named_profile_selection("wafer", &[]).expect("profile"),
    );
    let saved = std::fs::read_to_string(&env_path).unwrap_or_default();
    assert!(
        !saved.contains("waf-test-key-1234567890"),
        "logout must remove the stored key"
    );
}

#[test]
fn named_profile_without_api_key_env_is_skipped() {
    let _guard = ProfileEnvGuard::new(&["WAFER_TEST_KEY_ENV"]);
    // Overwrite config with a profile that has no api_key_env.
    let config_path = crate::config::Config::path().expect("config path");
    std::fs::write(
        &config_path,
        r#"
[providers.nokey]
type = "anthropic-compatible"
base_url = "https://example.invalid"
"#,
    )
    .expect("write config");
    crate::config::Config::invalidate_cache();

    let mut app = create_test_app();
    app.open_login_picker_inline();
    let entries = app.inline_interactive_state.as_ref().expect("picker");
    assert!(
        !entries
            .entries
            .iter()
            .any(|entry| entry.name.contains("nokey")),
        "profiles without api_key_env must not appear in the login picker"
    );
}

#[test]
fn remote_account_command_still_handles_login_logout_shape() {
    // Remote mode funnels /login and /logout through the same shared
    // parse in handle_auth_command (dispatch_local_command table), so the
    // remote path must accept the same input shapes without error.
    let _guard = ProfileEnvGuard::new(&["WAFER_TEST_KEY_ENV"]);
    let mut app = create_test_app();

    // The shared handler returns true (handled) for /login wafer; the actual
    // remote dispatch is exercised in key_handling via the same function.
    assert!(crate::tui::app::auth::handle_auth_command(&mut app, "/login wafer"));
    assert!(app.pending_login.is_some(), "profile login must stage");
    app.pending_login = None;
    assert!(crate::tui::app::auth::handle_auth_command(&mut app, "/logout wafer"));
    assert!(crate::tui::app::auth::handle_auth_command(&mut app, "/login not-a-provider"));
    assert!(app.pending_login.is_none());
}
