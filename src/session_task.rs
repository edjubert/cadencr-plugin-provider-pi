use serde_json::Value;

const CADENCR_META_KEY: &str = "com.cadencr";
const AUTO_NAME_PURPOSE: &str = "auto_name";
const FALLBACK_SYSTEM_PROMPT: &str =
    "Generate a short session title. Return only the title, with no explanation.";

#[derive(Debug, Clone)]
pub struct AutoNameTask {
    system_prompt: String,
}

impl AutoNameTask {
    pub fn from_session_params(params: &Value) -> Option<Self> {
        let metadata = params.get("_meta")?.get(CADENCR_META_KEY)?;
        if metadata.get("purpose").and_then(Value::as_str) != Some(AUTO_NAME_PURPOSE) {
            return None;
        }

        let system_prompt = metadata
            .get("systemPrompt")
            .and_then(Value::as_str)
            .filter(|prompt| !prompt.trim().is_empty())
            .unwrap_or(FALLBACK_SYSTEM_PROMPT)
            .to_string();
        Some(Self { system_prompt })
    }

    pub fn pi_args(&self) -> Vec<String> {
        [
            "--no-tools",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-context-files",
            "--append-system-prompt",
            "",
            "--thinking",
            "off",
            "--system-prompt",
        ]
        .into_iter()
        .map(str::to_string)
        .chain(std::iter::once(self.system_prompt.clone()))
        .collect()
    }
}
