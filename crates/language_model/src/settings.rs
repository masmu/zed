use std::time::Duration;

use anyhow::Result;
use gpui::AppContext;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings::{Settings, SettingsSources};

use crate::provider::{
    anthropic::AnthropicSettings,
    cloud::{self, ZedDotDevSettings},
    copilot_chat::CopilotChatSettings,
    google::GoogleSettings,
    ollama::OllamaSettings,
    open_ai::OpenAiSettings,
};

/// Initializes the language model settings.
pub fn init(cx: &mut AppContext) {
    AllLanguageModelSettings::register(cx);
}

#[derive(Default)]
pub struct AllLanguageModelSettings {
    pub anthropic: AnthropicSettings,
    pub ollama: OllamaSettings,
    pub openai: OpenAiSettings,
    pub zed_dot_dev: ZedDotDevSettings,
    pub google: GoogleSettings,
    pub copilot_chat: CopilotChatSettings,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct AllLanguageModelSettingsContent {
    pub anthropic: Option<AnthropicSettingsContent>,
    pub ollama: Option<OllamaSettingsContent>,
    pub openai: Option<OpenAiSettingsContent>,
    #[serde(rename = "zed.dev")]
    pub zed_dot_dev: Option<ZedDotDevSettingsContent>,
    pub google: Option<GoogleSettingsContent>,
    pub copilot_chat: Option<CopilotChatSettingsContent>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct AnthropicSettingsContent {
    pub api_url: Option<String>,
    pub low_speed_timeout_in_seconds: Option<u64>,
    pub available_models: Option<Vec<anthropic::Model>>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct OllamaSettingsContent {
    pub api_url: Option<String>,
    pub low_speed_timeout_in_seconds: Option<u64>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct OpenAiSettingsContent {
    pub api_url: Option<String>,
    pub low_speed_timeout_in_seconds: Option<u64>,
    pub available_models: Option<Vec<open_ai::Model>>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct GoogleSettingsContent {
    pub api_url: Option<String>,
    pub low_speed_timeout_in_seconds: Option<u64>,
    pub available_models: Option<Vec<google_ai::Model>>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct ZedDotDevSettingsContent {
    available_models: Option<Vec<cloud::AvailableModel>>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct CopilotChatSettingsContent {
    low_speed_timeout_in_seconds: Option<u64>,
}

impl settings::Settings for AllLanguageModelSettings {
    const KEY: Option<&'static str> = Some("language_models");

    type FileContent = AllLanguageModelSettingsContent;

    fn load(sources: SettingsSources<Self::FileContent>, _: &mut AppContext) -> Result<Self> {
        fn merge<T>(target: &mut T, value: Option<T>) {
            if let Some(value) = value {
                *target = value;
            }
        }

        let mut settings = AllLanguageModelSettings::default();

        for value in sources.defaults_and_customizations() {
            merge(
                &mut settings.anthropic.api_url,
                value.anthropic.as_ref().and_then(|s| s.api_url.clone()),
            );
            if let Some(low_speed_timeout_in_seconds) = value
                .anthropic
                .as_ref()
                .and_then(|s| s.low_speed_timeout_in_seconds)
            {
                settings.anthropic.low_speed_timeout =
                    Some(Duration::from_secs(low_speed_timeout_in_seconds));
            }
            merge(
                &mut settings.anthropic.available_models,
                value
                    .anthropic
                    .as_ref()
                    .and_then(|s| s.available_models.clone()),
            );

            merge(
                &mut settings.ollama.api_url,
                value.ollama.as_ref().and_then(|s| s.api_url.clone()),
            );
            if let Some(low_speed_timeout_in_seconds) = value
                .ollama
                .as_ref()
                .and_then(|s| s.low_speed_timeout_in_seconds)
            {
                settings.ollama.low_speed_timeout =
                    Some(Duration::from_secs(low_speed_timeout_in_seconds));
            }

            merge(
                &mut settings.openai.api_url,
                value.openai.as_ref().and_then(|s| s.api_url.clone()),
            );
            if let Some(low_speed_timeout_in_seconds) = value
                .openai
                .as_ref()
                .and_then(|s| s.low_speed_timeout_in_seconds)
            {
                settings.openai.low_speed_timeout =
                    Some(Duration::from_secs(low_speed_timeout_in_seconds));
            }
            merge(
                &mut settings.openai.available_models,
                value
                    .openai
                    .as_ref()
                    .and_then(|s| s.available_models.clone()),
            );

            merge(
                &mut settings.zed_dot_dev.available_models,
                value
                    .zed_dot_dev
                    .as_ref()
                    .and_then(|s| s.available_models.clone()),
            );

            merge(
                &mut settings.google.api_url,
                value.google.as_ref().and_then(|s| s.api_url.clone()),
            );
            if let Some(low_speed_timeout_in_seconds) = value
                .google
                .as_ref()
                .and_then(|s| s.low_speed_timeout_in_seconds)
            {
                settings.google.low_speed_timeout =
                    Some(Duration::from_secs(low_speed_timeout_in_seconds));
            }
            merge(
                &mut settings.google.available_models,
                value
                    .google
                    .as_ref()
                    .and_then(|s| s.available_models.clone()),
            );

            if let Some(low_speed_timeout) = value
                .copilot_chat
                .as_ref()
                .and_then(|s| s.low_speed_timeout_in_seconds)
            {
                settings.copilot_chat.low_speed_timeout =
                    Some(Duration::from_secs(low_speed_timeout));
            }
        }

        Ok(settings)
    }
}
