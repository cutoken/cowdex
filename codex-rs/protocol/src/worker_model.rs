//! User-selected worker models, including an optional provider and reasoning effort.

use crate::openai_models::ReasoningEffort;

/// A worker selector; provider-qualified model IDs are passed unchanged to that provider.
#[derive(Debug, PartialEq)]
pub struct WorkerModel<'a> {
    pub provider: Option<&'a str>,
    pub model: &'a str,
    pub effort: Option<ReasoningEffort>,
}

#[cfg(test)]
#[path = "worker_model_tests.rs"]
mod tests;

impl<'a> WorkerModel<'a> {
    pub fn parse(value: &'a str) -> Result<Self, String> {
        let (provider, value) = match value.split_once(':') {
            Some((provider @ ("zai" | "openrouter"), model)) => (Some(provider), model),
            _ => (None, value),
        };
        let (model, effort) = match value.rsplit_once(':') {
            Some((model, "low")) => (model, Some(ReasoningEffort::Low)),
            Some((model, "medium")) => (model, Some(ReasoningEffort::Medium)),
            Some((model, "high")) => (model, Some(ReasoningEffort::High)),
            Some((model, "xhigh")) => (model, Some(ReasoningEffort::XHigh)),
            Some((model, "max")) => (model, Some(ReasoningEffort::Max)),
            Some((model, "")) if provider.is_none() && !model.is_empty() => (model, None),
            Some(_) if provider.is_none() => {
                return Err("expected MODEL[:EFFORT] or PROVIDER:MODEL[:EFFORT]; effort: low, medium, high, xhigh, max".to_string());
            }
            _ => (value, None),
        };
        if provider.is_some() && model.trim().is_empty() {
            return Err("a provider-qualified worker requires a model name".to_string());
        }
        if provider.is_none() && model.contains(':') {
            return Err("expected MODEL[:EFFORT] or PROVIDER:MODEL[:EFFORT]".to_string());
        }
        let model = if provider.is_none() {
            match model {
                "" | "sol" => "gpt-6.1-sol",
                "astra" => "gpt-6-astra",
                "luna" => "gpt-6-luna",
                model => model,
            }
        } else {
            model
        };
        Ok(Self {
            provider,
            model,
            effort,
        })
    }

    pub fn qualified_model(&self) -> String {
        match self.provider {
            Some(provider) => format!("{provider}:{}", self.model),
            None => self.model.to_string(),
        }
    }
}
