use anyhow::{Context, Result};
use serde_json::Value;

use super::prompt;
use super::review::ReviewResult;
use crate::diff::change::{DiffResult, SemanticChange};

#[derive(Debug, Clone)]
pub enum LlmProvider {
    Anthropic { model: String },
    OpenAi { model: String },
}

pub struct LlmClient {
    api_key: String,
    provider: LlmProvider,
    http: reqwest::Client,
}

impl LlmClient {
    pub fn new(api_key: String, provider: LlmProvider) -> Self {
        Self {
            api_key,
            provider,
            http: reqwest::Client::new(),
        }
    }

    pub fn from_config(api_key: String, provider_name: &str, model: Option<String>) -> Self {
        let provider = match provider_name {
            "openai" => LlmProvider::OpenAi {
                model: model.unwrap_or_else(|| "gpt-4o".to_string()),
            },
            _ => LlmProvider::Anthropic {
                model: model.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
            },
        };
        Self::new(api_key, provider)
    }

    pub async fn review_change(&self, change: &SemanticChange) -> Result<ReviewResult> {
        let prompt_text = prompt::build_change_review_prompt(change);
        let response = self.send_prompt(&prompt_text).await?;
        parse_review_response(&response)
    }

    pub async fn summarize_diff(&self, result: &DiffResult) -> Result<ReviewResult> {
        let prompt_text = prompt::build_summary_prompt(result);
        let response = self.send_prompt(&prompt_text).await?;
        parse_review_response(&response)
    }

    async fn send_prompt(&self, prompt_text: &str) -> Result<String> {
        match &self.provider {
            LlmProvider::Anthropic { model } => {
                let body = serde_json::json!({
                    "model": model,
                    "max_tokens": 2048,
                    "messages": [
                        {"role": "user", "content": prompt_text}
                    ]
                });

                let resp = self
                    .http
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .context("Failed to send request to Anthropic API")?;

                let json: Value = resp.json().await?;
                let text = json["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                Ok(text)
            }
            LlmProvider::OpenAi { model } => {
                let body = serde_json::json!({
                    "model": model,
                    "messages": [
                        {"role": "user", "content": prompt_text}
                    ],
                    "max_tokens": 2048
                });

                let resp = self
                    .http
                    .post("https://api.openai.com/v1/chat/completions")
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .context("Failed to send request to OpenAI API")?;

                let json: Value = resp.json().await?;
                let text = json["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                Ok(text)
            }
        }
    }
}

fn parse_review_response(response: &str) -> Result<ReviewResult> {
    // Try to extract JSON from the response (LLM might wrap it in markdown)
    let json_str = if let Some(start) = response.find('{') {
        if let Some(end) = response.rfind('}') {
            &response[start..=end]
        } else {
            response
        }
    } else {
        response
    };

    serde_json::from_str(json_str).or_else(|_| {
        // If parsing fails, return a basic review with the raw text
        Ok(ReviewResult {
            summary: response.to_string(),
            risk_level: super::review::RiskLevel::Low,
            key_observations: Vec::new(),
            potential_issues: Vec::new(),
            test_suggestions: Vec::new(),
        })
    })
}
