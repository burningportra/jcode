//! The `typesafe` tool: TypeSafe (System One) structured decisions.
//!
//! One call answers any number of typed questions against structured `state` —
//! noul (a 0..1 yes-probability), choice (pick one option from `criteria`),
//! score (an ordered rubric in `criteria`). Fail-soft: when the feature is
//! disabled, no API key is configured, or the service errors, the call returns
//! a descriptive error and the agent falls back to its own judgment and keeps
//! working.
//!
//! The client is built per call (reading the live config) rather than captured
//! at registration time, matching how the other base tools resolve their
//! settings at execution time and keeping `base_tools()` registration simple.

use super::{Tool, ToolContext, ToolOutput};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::config::TypesafeConfig;

/// The question types TypeSafe (System One) supports.
const QUESTION_TYPES: [&str; 3] = ["noul", "choice", "score"];

pub struct TypesafeTool;

impl TypesafeTool {
    pub fn new() -> Self {
        Self
    }
}

/// Parsed and validated tool input. A `BTreeMap` gives stable question id
/// ordering for the body and easy iteration during validation.
#[derive(Debug, PartialEq)]
struct TypesafeToolInput {
    state: Value,
    questions: BTreeMap<String, Value>,
}

impl<'de> Deserialize<'de> for TypesafeToolInput {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let v: Value = Value::deserialize(deserializer)?;
        let state = v
            .get("state")
            .cloned()
            .ok_or_else(|| serde::de::Error::missing_field("state"))?;
        let questions = match v.get("questions") {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(id, q)| (id.clone(), q.clone()))
                .collect::<BTreeMap<String, Value>>(),
            _ => {
                return Err(serde::de::Error::custom(
                    "questions must be an object mapping question id -> question spec",
                ));
            }
        };
        if questions.is_empty() {
            return Err(serde::de::Error::custom(
                "at least one question is required",
            ));
        }
        Ok(Self { state, questions })
    }
}

/// Parse and validate tool input, returning the wire request. Kept separate
/// from `execute` so the rules are testable without a live client.
fn prepare_request(cfg: &TypesafeConfig, input: Value) -> Result<jcode_typesafe::SystemOneRequest> {
    let params: TypesafeToolInput =
        serde_json::from_value(input).map_err(|e| anyhow!("could not parse tool input: {e}"))?;

    let invalid_type = params
        .questions
        .iter()
        .find(|(_, q)| {
            q.get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| !QUESTION_TYPES.contains(&t))
        })
        .map(|(id, _)| id.clone());
    if let Some(id) = invalid_type {
        anyhow::bail!(
            "question {id:?} has an invalid type; use one of: {}",
            QUESTION_TYPES.join(", ")
        );
    }
    let bad_instructions = params.questions.iter().find(|(_, q)| {
        !q.get("instructions")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
    });
    if let Some((id, _)) = bad_instructions {
        anyhow::bail!("question {id:?} needs a non-empty \"instructions\" string");
    }

    let mut questions = Map::new();
    for (id, q) in params.questions {
        questions.insert(id.clone(), normalize_question(&id, q)?);
    }

    Ok(jcode_typesafe::SystemOneRequest {
        state: params.state,
        model: cfg.model.clone(),
        questions,
    })
}

/// Build one question object in the TypeSafe wire shape.
///
/// Choice and Score require `criteria` (map vs ordered array). A string-array
/// `options` field is accepted as a legacy alias and rewritten so a caller
/// that followed the old tool schema does not 422 against the live API.
fn normalize_question(id: &str, q: Value) -> Result<Value> {
    let qtype = q
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let instructions = q
        .get("instructions")
        .cloned()
        .ok_or_else(|| anyhow!("question {id:?} needs a non-empty \"instructions\" string"))?;

    let mut spec = Map::new();
    spec.insert("type".into(), json!(qtype));
    spec.insert("instructions".into(), instructions);

    match qtype.as_str() {
        "noul" => {
            if let Some(criteria) = q.get("criteria") {
                spec.insert("criteria".into(), criteria.clone());
            }
        }
        "choice" => {
            spec.insert("criteria".into(), choice_criteria(id, &q)?);
        }
        "score" => {
            spec.insert("criteria".into(), score_criteria(id, &q)?);
        }
        _ => {}
    }

    Ok(Value::Object(spec))
}

fn choice_criteria(id: &str, q: &Value) -> Result<Value> {
    if let Some(Value::Object(map)) = q.get("criteria") {
        if map.is_empty() {
            anyhow::bail!("question {id:?} choice criteria must have at least one option");
        }
        return Ok(Value::Object(map.clone()));
    }
    if let Some(Value::Array(opts)) = q.get("options") {
        if opts.is_empty() {
            anyhow::bail!("question {id:?} choice options must have at least one option");
        }
        let mut map = Map::new();
        for opt in opts {
            let key = opt
                .as_str()
                .ok_or_else(|| anyhow!("question {id:?} choice options must be strings"))?;
            if key.trim().is_empty() {
                anyhow::bail!("question {id:?} choice options must be non-empty strings");
            }
            map.insert(key.to_string(), Value::Null);
        }
        return Ok(Value::Object(map));
    }
    anyhow::bail!(
        "question {id:?} of type choice needs \"criteria\" (option -> description map) \
         or \"options\" (string array)"
    )
}

fn score_criteria(id: &str, q: &Value) -> Result<Value> {
    let levels = match q.get("criteria") {
        Some(Value::Array(levels)) => levels,
        Some(_) => anyhow::bail!(
            "question {id:?} of type score needs \"criteria\" as an ordered array of at least two levels"
        ),
        None => match q.get("options") {
            Some(Value::Array(levels)) => levels,
            _ => anyhow::bail!(
                "question {id:?} of type score needs \"criteria\" (ordered level descriptions, at least two)"
            ),
        },
    };
    if levels.len() < 2 {
        anyhow::bail!("question {id:?} score criteria needs at least two levels");
    }
    Ok(Value::Array(levels.clone()))
}

fn build_client(cfg: &TypesafeConfig) -> jcode_typesafe::Client {
    jcode_typesafe::Client::new(jcode_typesafe::ClientOptions {
        api_key: cfg.resolved_api_key(),
        base_url: cfg.base_url.clone(),
        model: cfg.model.clone(),
        timeout: Duration::from_secs(cfg.timeout_secs.max(1)),
    })
}

#[async_trait]
impl Tool for TypesafeTool {
    fn name(&self) -> &str {
        "typesafe"
    }

    fn description(&self) -> &str {
        "TypeSafe: typed noul/choice/score judgment over JSON state; fail-soft on error."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["state", "questions"],
            "properties": {
                "state": {
                    "type": "object",
                    "description": "Structured facts to evaluate against, e.g. {\"command\":\"rm -rf x\",\"request\":\"delete x\"}."
                },
                "questions": {
                    "type": "object",
                    "minProperties": 1,
                    "description": "Map of short id to {type,instructions[,criteria]}. Choice and score require criteria. All questions evaluated in one call.",
                    "additionalProperties": {
                        "type": "object",
                        "required": ["type", "instructions"],
                        "properties": {
                            "type": { "type": "string", "enum": QUESTION_TYPES.to_vec(), "description": "noul=p(yes) 0..1; choice=pick one of criteria; score=position on criteria levels." },
                            "instructions": { "type": "string" },
                            "criteria": {
                                "description": "choice: option->description map (null description ok). score: ordered level strings, at least two. noul: optional {true,false} clarifications."
                            },
                            "options": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Legacy alias rewritten to criteria: choice option names, or score levels."
                            }
                        }
                    }
                }
            }
        })
    }

    async fn execute(&self, input: Value, _ctx: ToolContext) -> Result<ToolOutput> {
        let cfg = crate::config::config();
        if !cfg.typesafe.enabled {
            anyhow::bail!(
                "typesafe is disabled in the Jcode config (config `typesafe` section `enabled` \
                 is false); nothing was evaluated — proceed on your own judgment"
            );
        }
        if cfg.typesafe.resolved_api_key().is_none() {
            anyhow::bail!(
                "TypeSafe has no API key configured. Set `typesafe.api_key` in the config file \
                 or the {} environment variable, then retry the same call.",
                cfg.typesafe.api_key_env
            );
        }
        let request = prepare_request(&cfg.typesafe, input)?;
        match build_client(&cfg.typesafe).evaluate(&request).await {
            Ok(result) => {
                Ok(ToolOutput::new(result.format_answers()).with_title("TypeSafe answers"))
            }
            Err(jcode_typesafe::EvalError::NoApiKey) => {
                // Defensive: the preflight above already rejected the no-key case.
                // Kept so a future reorder cannot regress into a raw transport error.
                anyhow::bail!(
                    "TypeSafe has no API key configured; set typesafe.api_key or the {} environment variable",
                    cfg.typesafe.api_key_env
                )
            }
            Err(jcode_typesafe::EvalError::Service(message)) => anyhow::bail!(
                "TypeSafe service unavailable ({message}); continue on your own judgment"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TypesafeRiskConfig;

    fn cfg_with_key() -> TypesafeConfig {
        TypesafeConfig {
            api_key: Some("sk-test".into()),
            risk: TypesafeRiskConfig::default(),
            ..Default::default()
        }
    }

    fn noul_question() -> Value {
        json!({
            "should_run": {
                "type": "noul",
                "instructions": "Is this command a faithful, safe fulfillment of the user's explicit request?"
            }
        })
    }

    fn input_object(state: &str, questions: Value) -> Value {
        json!({ "state": serde_json::from_str::<Value>(state).unwrap(), "questions": questions })
    }

    #[test]
    fn noul_prepares_system_one_body() {
        let req = prepare_request(
            &cfg_with_key(),
            input_object(
                r#"{"command":"rm -rf ~/Downloads","requested_by_user":"delete my old downloads"}"#,
                noul_question(),
            ),
        )
        .expect("prepared should succeed when a key is configured");

        let value: Value = serde_json::to_value(&req).expect("request must serialize");
        assert_eq!(value["state"]["command"], "rm -rf ~/Downloads");
        assert_eq!(
            value["state"]["requested_by_user"],
            "delete my old downloads"
        );
        // Model comes from config, not a hardcoded alias.
        assert_eq!(value["model"], cfg_with_key().model);
        // Question passthrough; no extra "id" field is transmitted.
        let q = &value["questions"]["should_run"];
        assert_eq!(q["type"], "noul");
        assert!(q["instructions"].as_str().is_some());
        assert!(q.get("id").is_none(), "question id is a key, not a field");
        assert!(value.get("id").is_none(), "request id is code-side only");
    }

    #[test]
    fn choice_and_score_criteria_use_official_wire_shape() {
        let questions = json!({
            "verdict": {
                "type": "choice",
                "instructions": "Pick the safest response given the command and the request.",
                "criteria": {
                    "proceed": "Safe to run as issued",
                    "ask_user": "Needs confirmation",
                    "refuse": "Must not run"
                }
            },
            "risk": {
                "type": "score",
                "instructions": "How irreversible is this command?",
                "criteria": ["reversible", "hard to undo", "permanent"]
            }
        });
        let req = prepare_request(&cfg_with_key(), input_object(r#"{"a":1}"#, questions)).unwrap();

        let value: Value = serde_json::to_value(&req).unwrap();
        let criteria = value["questions"]["verdict"]["criteria"]
            .as_object()
            .expect("choice criteria map");
        assert_eq!(criteria.len(), 3);
        assert_eq!(criteria["proceed"], json!("Safe to run as issued"));
        assert!(value["questions"]["verdict"].get("options").is_none());
        let levels = value["questions"]["risk"]["criteria"]
            .as_array()
            .expect("score criteria array");
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], json!("reversible"));
    }

    #[test]
    fn legacy_options_rewrite_to_criteria() {
        let questions = json!({
            "verdict": {
                "type": "choice",
                "instructions": "Pick the safest response given the command and the request.",
                "options": ["proceed", "ask_user", "refuse"]
            },
            "risk": {
                "type": "score",
                "instructions": "Irreversibility.",
                "options": ["low", "high"]
            }
        });
        let req = prepare_request(&cfg_with_key(), input_object(r#"{"a":1}"#, questions)).unwrap();

        let value: Value = serde_json::to_value(&req).unwrap();
        let criteria = value["questions"]["verdict"]["criteria"]
            .as_object()
            .expect("rewritten choice criteria");
        assert_eq!(criteria.get("proceed"), Some(&Value::Null));
        assert_eq!(criteria.get("ask_user"), Some(&Value::Null));
        assert_eq!(criteria.get("refuse"), Some(&Value::Null));
        assert!(value["questions"]["verdict"].get("options").is_none());
        let levels = value["questions"]["risk"]["criteria"]
            .as_array()
            .expect("rewritten score criteria");
        assert_eq!(levels, &vec![json!("low"), json!("high")]);
        assert!(value["questions"]["risk"].get("options").is_none());
    }

    #[test]
    fn multiple_questions_in_one_call_serve_in_a_single_request() {
        let questions = json!({
            "p_safe": { "type": "noul", "instructions": "Is the target path explicitly named by the user?" },
            "verdict": {
                "type": "choice",
                "instructions": "proceed or ask",
                "criteria": { "proceed": null, "ask_user": null }
            },
            "risk": {
                "type": "score",
                "instructions": "How irreversible?",
                "criteria": ["reversible", "permanent"]
            }
        });
        let req = prepare_request(
            &cfg_with_key(),
            input_object(r#"{"command":"rm -rf ~/Downloads"}"#, questions),
        )
        .unwrap();

        assert_eq!(req.questions.len(), 3, "all questions in one body");
        assert!(req.questions.contains_key("p_safe"));
        assert!(req.questions.contains_key("verdict"));
        assert!(req.questions.contains_key("risk"));
    }

    #[test]
    fn choice_without_criteria_or_options_is_rejected() {
        let err = prepare_request(
            &cfg_with_key(),
            input_object(
                r#"{}"#,
                json!({ "q": { "type": "choice", "instructions": "pick one" } }),
            ),
        )
        .expect_err("choice needs criteria");
        assert!(err.to_string().contains("criteria"), "{err}");
    }

    #[test]
    fn score_without_enough_levels_is_rejected() {
        let err = prepare_request(
            &cfg_with_key(),
            input_object(
                r#"{}"#,
                json!({ "q": { "type": "score", "instructions": "rate it", "criteria": ["only one"] } }),
            ),
        )
        .expect_err("score needs two levels");
        assert!(err.to_string().contains("at least two"), "{err}");
    }

    #[test]
    fn bad_question_type_is_rejected_before_any_network_call() {
        let err = prepare_request(
            &cfg_with_key(),
            input_object(
                r#"{}"#,
                json!({ "q": { "type": "boolean", "instructions": "yes or no?" } }),
            ),
        )
        .expect_err("invalid type must be rejected");
        assert!(err.to_string().contains("invalid type"), "{err}");
    }

    #[test]
    fn blank_or_missing_instructions_are_rejected() {
        let err = prepare_request(
            &cfg_with_key(),
            input_object(
                r#"{}"#,
                json!({ "q": { "type": "noul", "instructions": "   " } }),
            ),
        )
        .expect_err("blank instructions must be rejected");
        assert!(err.to_string().contains("instructions"), "{err}");

        let err = prepare_request(
            &cfg_with_key(),
            input_object(r#"{}"#, json!({ "q": { "type": "noul" } })),
        )
        .expect_err("missing instructions must be rejected");
        assert!(err.to_string().contains("instructions"), "{err}");
    }

    #[test]
    fn empty_questions_object_rejected() {
        let err = prepare_request(&cfg_with_key(), input_object(r#"{}"#, json!({})))
            .expect_err("no questions must be rejected");
        assert!(err.to_string().contains("question"), "{err}");
    }
}
