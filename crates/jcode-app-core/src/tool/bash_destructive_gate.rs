//! The destructive-command gate for the `bash` tool (issue #604).
//!
//! Kept in its own file so the policy seam is easy to find and review: this is
//! the only thing standing between a model's `rm -rf` and the user's data.
//!
//! The gate is deterministic by default. When `[typesafe.risk]` is enabled, a
//! command that reaches the confirm-tier reflection prompt (destructive, but
//! not statically catastrophic, and not justified on this attempt) gets one
//! extra chance: a single System One `noul` probe scores
//! p(running this exact command serves the user's request). A decisive reading
//! admits or refuses the command; a middle-band reading, a service error, and a
//! missing API key all defer to the original reflection prompt, so the decision
//! is fail-soft on every non-decisive path. The catastrophic tier is a hard
//! `Deny` before the reflect branch, so TypeSafe can unlock a confirm-tier
//! command but can never unlock a catastrophic one.

use std::time::Duration;

/// Apply the destructive-command gate, returning refusal text when the command
/// must not run as-issued. `None` means "run it".
///
/// Stage 1 is a pure blast-radius assessment; stage 2 turns a `Confirm` verdict
/// into a reflection prompt that a blind retry cannot satisfy. Catastrophic
/// targets (`/`, `$HOME`, credential stores, device nodes) are denied outright.
/// See issue #604.
///
/// `typesafe` is the live TypeSafe config. Pass `None` (or a config with
/// `risk.enabled` false) for the fully deterministic path. When it is enabled,
/// the reflect branch consults one noul probe before falling back to the prompt.
pub(super) async fn destructive_command_refusal(
    command: &str,
    justification: Option<&str>,
    working_dir: Option<std::path::PathBuf>,
    typesafe: Option<&crate::config::TypesafeConfig>,
) -> Option<String> {
    let risk_ctx = jcode_command_risk::RiskContext::from_env(working_dir);
    let assessment = jcode_command_risk::assess(command, &risk_ctx);
    if assessment.level.runs_immediately() {
        return None;
    }

    let justification = jcode_command_risk::Justification {
        text: justification.map(str::to_string),
    };
    match jcode_command_risk::gate(&assessment, &justification) {
        jcode_command_risk::GateOutcome::Allow => None,
        jcode_command_risk::GateOutcome::Deny { reason } => {
            crate::logging::warn(&format!("[bash] denied destructive command: {command}"));
            Some(reason)
        }
        jcode_command_risk::GateOutcome::Reflect { prompt } => {
            // Only the confirm-tier, unjustified path reaches here. Catastrophic
            // is a hard Deny above, and a substantive justification is an Allow
            // above, so an adjudication can never unlock either.
            if let Some(cfg) = typesafe
                && cfg.risk.enabled
            {
                match typesafe_adjudication(cfg, command).await {
                    Adjudication::Allow => {
                        crate::logging::info(&format!(
                            "[bash] typesafe adjudication admitted a confirm-tier command: {command}"
                        ));
                        return None;
                    }
                    Adjudication::Refuse(refusal) => return Some(refusal),
                    Adjudication::NoDecision => {
                        // Middle band / service error / missing key: fall
                        // through to the deterministic reflection prompt.
                    }
                }
            }
            crate::logging::info(&format!(
                "[bash] destructive command held for justification: {command}"
            ));
            Some(prompt)
        }
    }
}

/// The yes/no question asked of System One for a confirm-tier destructive
/// command. The returned probability is read as p(running this exact command
/// serves the user's request).
const NOUL_INSTRUCTIONS: &str = "Is running this exact destructive command a faithful fulfillment of the user's explicit request? Answer yes only if the user explicitly asked for this action on these specific targets.";
/// Code-side question id used to read the answer back. It is not treated as a
/// provider wire field, only as the key under `questions`.
const NOUL_QUESTION_ID: &str = "serves_user_request";

/// What the gate should learn from a noul reading, relative to the thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoulVerdict {
    /// Reading is at or above `allow_above`: admit the command.
    Allow,
    /// Reading is at or below `require_below`: refuse the command.
    Refuse,
    /// Open band between the thresholds (or missing): defer to reflection.
    NoDecision,
}

/// Map a noul yes-probability to a verdict using the configured thresholds.
/// `allow_above` admits, `require_below` refuses, and the open band (or a
/// missing value) is a non-decision.
fn classify_noul(
    probability: Option<f64>,
    risk: &crate::config::TypesafeRiskConfig,
) -> NoulVerdict {
    let Some(p) = probability else {
        return NoulVerdict::NoDecision;
    };
    if p >= risk.allow_above {
        NoulVerdict::Allow
    } else if p <= risk.require_below {
        NoulVerdict::Refuse
    } else {
        NoulVerdict::NoDecision
    }
}

/// What the gate should learn from the TypeSafe probe.
#[derive(Debug)]
enum Adjudication {
    /// Let the command run.
    Allow,
    /// Refuse it with this text.
    Refuse(String),
    /// No decision: keep the deterministic reflection prompt.
    NoDecision,
}

/// Run one System One `noul` probe for a confirm-tier destructive command and
/// turn it into an [`Adjudication`]. Every non-decisive path — a middle-band
/// reading, a service error, a missing API key — returns
/// [`Adjudication::NoDecision`] so the original reflection prompt is the
/// fallback. A missing key is logged distinctly (an actionable config problem)
/// from a transient service error.
async fn typesafe_adjudication(cfg: &crate::config::TypesafeConfig, command: &str) -> Adjudication {
    let client = jcode_typesafe::Client::new(jcode_typesafe::ClientOptions {
        api_key: cfg.resolved_api_key(),
        base_url: cfg.base_url.clone(),
        model: cfg.model.clone(),
        timeout: Duration::from_secs(cfg.timeout_secs.max(1)),
    });
    let request = jcode_typesafe::SystemOneRequest::noul_only(
        serde_json::json!({ "command": command }),
        NOUL_QUESTION_ID,
        NOUL_INSTRUCTIONS,
    );
    // Use `evaluate` (not the `evaluate_noul` convenience) so a missing API key
    // is distinguishable from a transient service error; both fail soft below.
    let noul = client
        .evaluate(&request)
        .await
        .map(|result| result.noul(NOUL_QUESTION_ID));

    match noul {
        Ok(prob) => {
            let display = prob.map_or("n/a".to_string(), |p| format!("{p:.3}"));
            match classify_noul(prob, &cfg.risk) {
                NoulVerdict::Allow => {
                    crate::logging::info(&format!(
                        "typesafe gate: noul {display} >= {} -> allow",
                        cfg.risk.allow_above
                    ));
                    Adjudication::Allow
                }
                NoulVerdict::Refuse => {
                    let p = prob.unwrap_or(0.0);
                    crate::logging::info(&format!(
                        "typesafe gate: noul {display} <= {} -> refuse",
                        cfg.risk.require_below
                    ));
                    Adjudication::Refuse(format_refusal(command, p, &cfg.risk))
                }
                NoulVerdict::NoDecision => {
                    crate::logging::info(&format!(
                        "typesafe gate: noul {display} in ({}, {}) -> defer to reflection",
                        cfg.risk.require_below, cfg.risk.allow_above
                    ));
                    Adjudication::NoDecision
                }
            }
        }
        Err(jcode_typesafe::EvalError::NoApiKey) => {
            crate::logging::warn(&format!(
                "typesafe gate: [typesafe.risk] is enabled but no TypeSafe API key is configured. \
                 Set `typesafe.api_key` in the config file or the {env} environment variable, or \
                 disable `[typesafe.risk]`. Deferring to the deterministic reflection prompt.",
                env = cfg.api_key_env
            ));
            Adjudication::NoDecision
        }
        Err(jcode_typesafe::EvalError::Service(message)) => {
            crate::logging::warn(&format!(
                "typesafe gate: adjudication service error ({message}); deferring to the \
                 deterministic reflection prompt (fail-soft)"
            ));
            Adjudication::NoDecision
        }
    }
}

/// Refusal text for a low-noul (refuse) reading.
fn format_refusal(
    command: &str,
    probability: f64,
    risk: &crate::config::TypesafeRiskConfig,
) -> String {
    format!(
        "This command was not run. A TypeSafe (System One) adjudication scored the probability that this destructive command serves the user's request at {probability:.3}, which is at or below the configured require-below threshold ({require_below}). Do not re-issue this command: surface the risk to the user and ask how they want to proceed.\n\n```\n{command}\n```\n",
        require_below = risk.require_below
    )
}

/// The `bash` tool's JSON schema, including the `justification` field the
/// destructive-command gate consumes.
///
/// Lives beside the gate so the schema and the policy that reads it stay in
/// sync, and so bash.rs stays inside the code-size budget.
pub(super) fn bash_parameters_schema() -> serde_json::Value {
    let cmd_desc = if cfg!(windows) {
        "The Windows command to execute via cmd.exe. Use cmd.exe syntax and quoting, not Bash syntax."
    } else {
        "The bash command to execute. Put large temp files under `$JCODE_SCRATCH_DIR`, not `/tmp`."
    };
    serde_json::json!({
        "type": "object",
        "required": ["command"],
        "properties": {
            "intent": crate::tool::intent_schema_property(),
            "command": {
                "type": "string",
                "description": cmd_desc
            },
            "timeout": {
                "type": "integer",
                "description": "Timeout in MILLISECONDS (not seconds), e.g. 600000 = 10min; kills with exit 124. Omit for no timeout."
            },
            "run_in_background": {
                "type": "boolean",
                "description": "Run in background. Emit `JCODE_PROGRESS {json}` lines for progress reporting."
            },
            "notify": {
                "type": "boolean",
                "description": "Notify on completion."
            },
            "wake": {
                "type": "boolean",
                "description": "Wake on completion."
            },
            "stall_wake_seconds": {
                "type": "integer",
                "description": "With run_in_background: wake the agent after this many seconds with no output/progress (min 30, resets on activity)."
            },
            "justification": {
                "type": "string",
                "description": "Only when re-issuing a command the destructive gate refused; explain which user request it serves."
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn risk_cfg(
        enabled: bool,
        allow_above: f64,
        require_below: f64,
    ) -> crate::config::TypesafeConfig {
        crate::config::TypesafeConfig {
            risk: crate::config::TypesafeRiskConfig {
                enabled,
                allow_above,
                require_below,
            },
            ..Default::default()
        }
    }

    // A genuine confirm-tier destructive command: a `$`-substituted target the
    // static parser cannot resolve, so it is `Confirm` (not `Low` or
    // `Catastrophic`) and reaches the reflect branch. Verified against the real
    // classifier: `rm -rf $CLOUD/data` assesses as `Confirm`. It is a constant
    // string, so no filesystem fixture or env mutation is needed.
    const CONFIRM_COMMAND: &str = "rm -rf $CLOUD/data";

    fn enabled_cfg_at(
        base_url: &str,
        allow_above: f64,
        require_below: f64,
    ) -> crate::config::TypesafeConfig {
        crate::config::TypesafeConfig {
            enabled: true,
            api_key: Some("test-key".into()),
            // An unset var name forces resolution of the value from above,
            // i.e. the api_key field, instead of the real TYPESAFE_API_KEY.
            api_key_env: "JCODE_TEST_TYPESAFE_GATE_KEY_UNSET".into(),
            base_url: base_url.into(),
            risk: crate::config::TypesafeRiskConfig {
                enabled: true,
                allow_above,
                require_below,
            },
            ..Default::default()
        }
    }

    fn noul_body(value: f64) -> String {
        serde_json::json!({
            "model": "jev-latest",
            "answers": {
                "serves_user_request": { "type": "noul", "noul": value }
            },
            "usage": { "input_tokens": 1, "output_tokens": 1 }
        })
        .to_string()
    }

    // A minimal HTTP server that replays one fixed JSON body with 200. The
    // client sends exactly one noul request, so a single static reply is all
    // the decision paths (allow / refuse / middle-band) need.
    struct MockServer {
        body: String,
    }

    impl MockServer {
        async fn bind(self) -> String {
            let body = self.body;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock");
            let addr = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    use tokio::io::AsyncWriteExt;
                    loop {
                        let (mut stream, _peer) = match listener.accept().await {
                            Ok(pair) => pair,
                            Err(_) => break,
                        };
                        // A fresh copy of the reply for each connection.
                        let body = body.clone();
                        tokio::spawn(async move {
                            // Read up to (and including) the blank line so the
                            // request is consumed; the body is not asserted.
                            let mut buf: Vec<u8> = Vec::new();
                            let mut chunk = [0u8; 4096];
                            use tokio::io::AsyncReadExt;
                            loop {
                                match stream.read(&mut chunk).await {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        buf.extend_from_slice(&chunk[..n]);
                                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                            break;
                                        }
                                        if buf.len() > 1 << 20 {
                                            break;
                                        }
                                    }
                                    Err(_) => return,
                                }
                            }
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        });
                    }
                });
            });
            format!("http://{addr}")
        }
    }

    // --- classify_noul: the pure threshold table ----------------------------

    #[test]
    fn classify_noul_threshold_table() {
        let risk = crate::config::TypesafeConfig::default().risk; // 0.7 / 0.5
        assert_eq!(classify_noul(Some(0.8), &risk), NoulVerdict::Allow);
        assert_eq!(classify_noul(Some(0.7), &risk), NoulVerdict::Allow); // boundary
        assert_eq!(classify_noul(Some(0.55), &risk), NoulVerdict::NoDecision);
        assert_eq!(classify_noul(Some(0.65), &risk), NoulVerdict::NoDecision);
        assert_eq!(classify_noul(Some(0.5), &risk), NoulVerdict::Refuse); // boundary
        assert_eq!(classify_noul(Some(0.1), &risk), NoulVerdict::Refuse);
        assert_eq!(classify_noul(None, &risk), NoulVerdict::NoDecision);
    }

    // --- typesafe_adjudication: decision mapping over a live client --------

    #[tokio::test]
    async fn adjudication_allows_a_high_noul() {
        let base = MockServer {
            body: noul_body(0.92),
        }
        .bind()
        .await;
        let adj = typesafe_adjudication(&enabled_cfg_at(&base, 0.7, 0.5), "rm -rf X/data").await;
        assert!(matches!(adj, Adjudication::Allow), "{adj:?}");
    }

    #[tokio::test]
    async fn adjudication_refuses_a_low_noul() {
        let base = MockServer {
            body: noul_body(0.10),
        }
        .bind()
        .await;
        let command = "rm -rf X/data";
        let adj = typesafe_adjudication(&enabled_cfg_at(&base, 0.7, 0.5), command).await;
        match adj {
            Adjudication::Refuse(text) => {
                assert!(
                    text.contains("TypeSafe (System One) adjudication"),
                    "{text}"
                );
                assert!(text.contains(command), "{text}");
            }
            other => panic!("expected Refuse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn adjudication_defers_a_middle_noul() {
        let base = MockServer {
            body: noul_body(0.60),
        }
        .bind()
        .await;
        let adj = typesafe_adjudication(&enabled_cfg_at(&base, 0.7, 0.5), "rm -rf X/data").await;
        assert!(matches!(adj, Adjudication::NoDecision), "{adj:?}");
    }

    #[tokio::test]
    async fn adjudication_without_a_key_is_no_decision_before_any_io() {
        // Closed port: if the client ever made a request it would time out.
        // NoApiKey must short-circuit before any I/O, so this returns fast.
        let cfg = crate::config::TypesafeConfig {
            api_key: None,
            api_key_env: "JCODE_TEST_TYPESAFE_GATE_KEY_UNSET".into(),
            base_url: "http://127.0.0.1:1".into(),
            ..enabled_cfg_at("http://127.0.0.1:1", 0.7, 0.5)
        };
        let adj = typesafe_adjudication(&cfg, "rm -rf X/data").await;
        assert!(matches!(adj, Adjudication::NoDecision), "{adj:?}");
    }

    #[tokio::test]
    async fn adjudication_on_service_error_is_no_decision() {
        // A key against a closed port: every attempt is a refused (transient)
        // connection and the client exhausts its retries as a Service error.
        let cfg = crate::config::TypesafeConfig {
            timeout_secs: 1,
            ..enabled_cfg_at("http://127.0.0.1:1", 0.7, 0.5)
        };
        let adj = typesafe_adjudication(&cfg, "rm -rf X/data").await;
        assert!(matches!(adj, Adjudication::NoDecision), "{adj:?}");
    }

    // --- gate routing: adjudication only on the reflect branch --------------

    #[tokio::test]
    async fn gate_allows_a_confirm_command_when_noul_is_high() {
        let base = MockServer {
            body: noul_body(0.95),
        }
        .bind()
        .await;
        let cfg = enabled_cfg_at(&base, 0.7, 0.5);
        // Sanity: under the deterministic path (risk disabled) the same command is
        // held for reflection, proving CONFIRM_COMMAND is genuinely confirm-tier,
        // so the Allow below is due to the high noul rather than the command never
        // being gated.
        assert!(
            destructive_command_refusal(
                CONFIRM_COMMAND,
                None,
                None,
                Some(&risk_cfg(false, 0.7, 0.5))
            )
            .await
            .is_some(),
            "CONFIRM_COMMAND should be held under the deterministic path"
        );
        let refusal = destructive_command_refusal(CONFIRM_COMMAND, None, None, Some(&cfg)).await;
        assert!(
            refusal.is_none(),
            "a high noul should admit the confirm command, got: {refusal:?}"
        );
    }

    #[tokio::test]
    async fn gate_defers_to_reflection_when_noul_is_middle() {
        let base = MockServer {
            body: noul_body(0.60),
        }
        .bind()
        .await;
        let refusal = destructive_command_refusal(
            CONFIRM_COMMAND,
            None,
            None,
            Some(&enabled_cfg_at(&base, 0.7, 0.5)),
        )
        .await
        .expect("a middle noul should defer to the reflection prompt");
        assert!(
            refusal.contains("justification"),
            "expected the deterministic reflection prompt, got: {refusal}"
        );
    }

    #[tokio::test]
    async fn gate_is_deterministic_when_risk_is_disabled() {
        // `enabled = false`: no probe, so no network is involved and the plain
        // reflection prompt is produced.
        let refusal = destructive_command_refusal(
            CONFIRM_COMMAND,
            None,
            None,
            Some(&risk_cfg(false, 0.7, 0.5)),
        )
        .await
        .expect("disabled risk must still hold the command");
        assert!(
            refusal.contains("justification"),
            "disabled risk must use the deterministic prompt, got: {refusal}"
        );
    }

    #[tokio::test]
    async fn gate_refuses_when_noul_is_low() {
        let base = MockServer {
            body: noul_body(0.10),
        }
        .bind()
        .await;
        let refusal = destructive_command_refusal(
            CONFIRM_COMMAND,
            None,
            None,
            Some(&enabled_cfg_at(&base, 0.7, 0.5)),
        )
        .await
        .expect("a low noul should refuse the command");
        assert!(
            refusal.contains("TypeSafe (System One) adjudication"),
            "expected the low-noul refusal, got: {refusal}"
        );
    }

    #[tokio::test]
    async fn adjudication_cannot_unlock_a_catastrophic_target() {
        // `/etc` is a protected system path, so `rm -rf /etc` is `Catastrophic`
        // (a hard Deny) even when the mock answers a max-out noul of 0.99.
        let base = MockServer {
            body: noul_body(0.99),
        }
        .bind()
        .await;
        let cfg = enabled_cfg_at(&base, 0.7, 0.5);
        let refusal = destructive_command_refusal("rm -rf /etc", None, None, Some(&cfg)).await;
        let refusal = refusal.expect("catastrophic must stay denied even at noul 0.99");
        assert!(
            refusal.contains("blocked"),
            "expected an outright block, got: {refusal}"
        );
    }
}
