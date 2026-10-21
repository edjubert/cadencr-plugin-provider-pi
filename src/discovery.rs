//! Pre-session model discovery via `pi --mode rpc`.
//!
//! Launches a disposable Pi RPC process in the requested workspace,
//! calls `get_available_models` and `get_state`, then builds the ACP v1
//! `SessionConfigOption` array with a `model` select option and an optional
//! `thinking` select option.
//!
//! The host terminates `models` after 10 seconds, so discovery stays
//! under an 8-second budget.

use agent_client_protocol::schema::v1::{
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    SessionConfigSelectOptions,
};
use std::path::Path;
use std::time::Duration;

use crate::pi_rpc::{spawn, PiModel, PiRpc, PiState};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(8);

/// Discover model options for the workspace at `cwd`.
pub fn discover_model_options(cwd: &Path) -> Result<Vec<SessionConfigOption>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create runtime: {e}"))?;

    runtime.block_on(async {
        tokio::time::timeout(DISCOVERY_TIMEOUT, discover_inner(cwd))
            .await
            .map_err(|_| "pi model discovery timed out after 8 seconds".to_string())?
    })
}

async fn discover_inner(cwd: &Path) -> Result<Vec<SessionConfigOption>, String> {
    let rpc = spawn(cwd, None, true, &[])?;
    let options = load_config_options(&rpc).await;
    rpc.shutdown().await;
    options
}

/// Query pi for models, state and the current model's thinking levels, and
/// build the ACP config options. Single source of truth for `models`
/// discovery and for live `session/new|resume|set_config_option`.
pub async fn load_config_options(rpc: &PiRpc) -> Result<Vec<SessionConfigOption>, String> {
    let mut models = rpc.get_available_models().await?;
    if models.is_empty() {
        return Err(
            "pi returned no available models. Check your Pi configuration \
             and provider credentials (~/.pi/agent/settings.json)."
                .to_string(),
        );
    }
    let state = rpc.get_state().await?;
    let levels = rpc.get_available_thinking_levels().await?;
    apply_thinking_levels(&mut models, &state, levels);
    Ok(build_session_config_options(&models, Some(&state)))
}

/// Attach the thinking levels to the model they were fetched for (the
/// current one). Pi only reports levels for the current model.
fn apply_thinking_levels(models: &mut [PiModel], state: &PiState, levels: Vec<String>) {
    let current = state.model.as_ref().map(PiModel::acp_model_id);
    let target = models
        .iter_mut()
        .find(|m| Some(m.acp_model_id()) == current);
    if let Some(model) = target {
        model.thinking_levels = levels;
    }
}

/// Build the full set of session config options from Pi models and state.
///
/// This is the single source of truth used by both `models` discovery and
/// live `session/new` / `session/resume` responses.
pub fn build_session_config_options(
    models: &[PiModel],
    state: Option<&PiState>,
) -> Vec<SessionConfigOption> {
    let current_model_id = state
        .and_then(|s| s.model.as_ref())
        .map(|m| m.acp_model_id())
        .unwrap_or_else(|| models[0].acp_model_id());

    let model_option = build_model_option(models, &current_model_id);
    let mut options = vec![model_option];

    // Add thinking level option if any model supports multiple levels.
    if let Some(thinking_option) = build_thinking_option(models, state) {
        options.push(thinking_option);
    }

    options
}

/// Build the `model` select option from Pi's available models.
pub fn build_model_option(models: &[PiModel], current_model_id: &str) -> SessionConfigOption {
    let choices: Vec<SessionConfigSelectOption> = models
        .iter()
        .map(|m| {
            let label = match &m.provider {
                Some(p) if !p.is_empty() => format!("{}: {}", p, m.name),
                _ => m.name.clone(),
            };
            let mut description = String::new();
            if m.supports_images {
                description.push_str("Supports images. ");
            }
            if let Some(max_ctx) = m.max_context {
                description.push_str(&format!("Max context: {} tokens.", max_ctx));
            }
            let description = description.trim().to_string();

            let mut opt = SessionConfigSelectOption::new(m.acp_model_id(), label);
            if !description.is_empty() {
                opt = opt.description(description);
            }
            opt
        })
        .collect();

    let current_value = if models.iter().any(|m| m.acp_model_id() == current_model_id) {
        current_model_id.to_string()
    } else {
        models[0].acp_model_id()
    };

    SessionConfigOption::select(
        "model",
        "Model",
        current_value,
        SessionConfigSelectOptions::Ungrouped(choices),
    )
    .description("Select the model before starting the session")
    .category(SessionConfigOptionCategory::Model)
}

/// Build the `thinking` select option if models expose thinking levels.
pub fn build_thinking_option(
    models: &[PiModel],
    state: Option<&PiState>,
) -> Option<SessionConfigOption> {
    // Use the thinking levels of the current model (Pi only reports levels
    // for the current model). Fall back to the first model when no state.
    let current_id = state
        .and_then(|s| s.model.as_ref())
        .map(PiModel::acp_model_id)
        .or_else(|| models.first().map(PiModel::acp_model_id))?;
    let mut all_levels: Vec<String> = models
        .iter()
        .find(|m| m.acp_model_id() == current_id)
        .map(|m| m.thinking_levels.clone())
        .unwrap_or_default();
    if all_levels.len() < 2 {
        return None;
    }

    // Sort levels in a sensible order.
    let level_order = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    all_levels.sort_by_key(|lvl| {
        level_order
            .iter()
            .position(|&o| o == lvl.as_str())
            .unwrap_or(usize::MAX)
    });

    let current_level = state
        .and_then(|s| s.thinking_level.clone())
        .unwrap_or_else(|| "medium".to_string());

    let current_level = if all_levels.contains(&current_level) {
        current_level
    } else {
        all_levels.first().cloned().unwrap_or_default()
    };

    let choices: Vec<SessionConfigSelectOption> = all_levels
        .iter()
        .map(|level| SessionConfigSelectOption::new(level.clone(), capitalize(level)))
        .collect();

    Some(
        SessionConfigOption::select(
            "thinking",
            "Thinking Level",
            current_level,
            SessionConfigSelectOptions::Ungrouped(choices),
        )
        .description("Select the thinking/reasoning effort level")
        .category(SessionConfigOptionCategory::ThoughtLevel),
    )
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pi_rpc::PiModel;

    fn sample_models() -> Vec<PiModel> {
        vec![
            PiModel {
                id: "gpt-5.6-luna".to_string(),
                name: "GPT 5.6 Luna".to_string(),
                provider: Some("openai-codex".to_string()),
                thinking_levels: vec!["low".into(), "medium".into(), "high".into(), "max".into()],
                default_thinking_level: Some("medium".into()),
                supports_images: true,
                max_context: Some(200_000),
            },
            PiModel {
                id: "claude-sonnet-4".to_string(),
                name: "Claude Sonnet 4".to_string(),
                provider: Some("anthropic".to_string()),
                thinking_levels: vec!["low".into(), "high".into()],
                default_thinking_level: None,
                supports_images: true,
                max_context: Some(180_000),
            },
        ]
    }

    #[test]
    fn builds_model_option_with_all_models() {
        let models = sample_models();
        let option = build_model_option(&models, "openai-codex/gpt-5.6-luna");
        assert_eq!(&*option.id.0, "model");
        assert_eq!(option.category, Some(SessionConfigOptionCategory::Model));

        if let agent_client_protocol::schema::v1::SessionConfigKind::Select(select) = &option.kind {
            assert_eq!(&*select.current_value.0, "openai-codex/gpt-5.6-luna");
            if let SessionConfigSelectOptions::Ungrouped(choices) = &select.options {
                assert_eq!(choices.len(), 2);
                assert_eq!(&*choices[0].value.0, "openai-codex/gpt-5.6-luna");
                assert_eq!(choices[0].name, "openai-codex: GPT 5.6 Luna");
                assert_eq!(&*choices[1].value.0, "anthropic/claude-sonnet-4");
            } else {
                panic!("expected ungrouped options");
            }
        } else {
            panic!("expected select kind");
        }
    }

    #[test]
    fn falls_back_to_first_model_when_current_not_found() {
        let models = sample_models();
        let option = build_model_option(&models, "nonexistent/model");
        if let agent_client_protocol::schema::v1::SessionConfigKind::Select(select) = &option.kind {
            assert_eq!(&*select.current_value.0, "openai-codex/gpt-5.6-luna");
        } else {
            panic!("expected select kind");
        }
    }

    #[test]
    fn thinking_option_uses_current_model_levels_only() {
        let mut models = sample_models();
        let current = models[0].acp_model_id();
        models[0].thinking_levels = vec!["off".into(), "low".into(), "high".into()];
        models[1].thinking_levels = vec!["off".into(), "max".into()];
        let state = PiState {
            model: Some(models[0].clone()),
            thinking_level: Some("low".to_string()),
            session_id: None,
            session_name: None,
        };
        let option = build_thinking_option(&models, Some(&state)).expect("thinking option");
        let json = serde_json::to_value(&option).unwrap();
        let values: Vec<&str> = json["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["value"].as_str().unwrap())
            .collect();
        assert_eq!(values, vec!["off", "low", "high"]);
        assert_eq!(json["currentValue"], "low");
        assert_eq!(models[0].acp_model_id(), current);
    }

    #[test]
    fn no_thinking_option_when_single_level() {
        let models = vec![PiModel {
            id: "model-a".to_string(),
            name: "Model A".to_string(),
            provider: Some("test".to_string()),
            thinking_levels: vec!["off".into()],
            default_thinking_level: None,
            supports_images: false,
            max_context: None,
        }];
        assert!(build_thinking_option(&models, None).is_none());
    }

    #[test]
    fn apply_thinking_levels_targets_the_current_model() {
        let mut models = sample_models();
        // The non-current model starts with no levels so we can assert it is
        // left untouched.
        models[0].thinking_levels = vec![];
        let state = PiState {
            model: Some(models[1].clone()),
            thinking_level: None,
            session_id: None,
            session_name: None,
        };
        apply_thinking_levels(&mut models, &state, vec!["off".into(), "high".into()]);
        assert!(models[0].thinking_levels.is_empty());
        assert_eq!(models[1].thinking_levels, vec!["off", "high"]);
    }

    #[test]
    fn serializes_to_acp_json() {
        let models = sample_models();
        let option = build_model_option(&models, "openai-codex/gpt-5.6-luna");
        let json = serde_json::to_value(&option).unwrap();
        assert_eq!(json["id"], "model");
        assert_eq!(json["category"], "model");
        assert_eq!(json["type"], "select");
        assert!(json["options"].is_array());
    }

    #[test]
    fn capitalize_works() {
        assert_eq!(capitalize("low"), "Low");
        assert_eq!(capitalize("max"), "Max");
        assert_eq!(capitalize(""), "");
    }
}
