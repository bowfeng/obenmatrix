//! Setup panel — interactive setup wizard in TUI form.

use super::{KeyAction, Panel};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use oben_config::AppConfig;
use oben_models::ProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    Welcome,
    Provider,
    ModelName,
    ApiKey,
    MaxIterations,
    Compression,
    Complete,
}

pub struct SetupPanel {
    pub step: SetupStep,
    pub config: AppConfig,
    pub provider_selected: usize,
    pub model_input: String,
    pub api_key_input: String,
    pub max_iter_input: String,
    pub compression_input: String,
    pub help: String,
}

impl SetupPanel {
    pub fn new() -> Self {
        Self {
            step: SetupStep::Welcome,
            config: AppConfig::default(),
            provider_selected: 0,
            model_input: "qwen/qwen3-235b:free".to_string(),
            api_key_input: String::new(),
            max_iter_input: "50".to_string(),
            compression_input: "summary".to_string(),
            help: "Welcome to ObenAgent Setup! Press Enter to continue.".to_string(),
        }
    }

    pub fn set_config(&mut self, config: AppConfig) {
        let provider_idx = match config.model.kind {
            ProviderKind::OpenRouter => 0,
            ProviderKind::OpenAI => 1,
            ProviderKind::Anthropic => 2,
            ProviderKind::Bedrock => 3,
            ProviderKind::Gemini => 4,
            ProviderKind::LMStudio => 5,
            _ => 5, // All new providers default to LMStudio/index 5
        };
        let max_iter_str = config
            .max_iterations
            .map(|v| v.to_string())
            .unwrap_or("50".to_string());
        let compression = config.context.compression.clone();
        let model = config.model.model.clone();
        let api_key = config.model.api_key.clone().unwrap_or_default();
        self.provider_selected = provider_idx;
        self.config = config;
        self.model_input = model;
        self.api_key_input = api_key;
        self.max_iter_input = max_iter_str;
        self.compression_input = compression;
    }

    fn providers(&self) -> Vec<&'static str> {
        vec![
            "OpenRouter",
            "OpenAI",
            "Anthropic",
            "Bedrock",
            "Gemini",
            "LMStudio (local)",
            "Custom endpoint",
        ]
    }

    fn next_step(&mut self) {
        self.step = match self.step {
            SetupStep::Welcome => SetupStep::Provider,
            SetupStep::Provider => SetupStep::ModelName,
            SetupStep::ModelName => SetupStep::ApiKey,
            SetupStep::ApiKey => SetupStep::MaxIterations,
            SetupStep::MaxIterations => SetupStep::Compression,
            _ => SetupStep::Complete,
        };
        self.update_help();
    }

    fn update_help(&mut self) {
        self.help = match self.step {
            SetupStep::Welcome => {
                "Welcome to ObenAgent Setup!\n\nThis wizard will help you configure the agent.\nPress Enter to continue. Press q to cancel.".to_string()
            }
            SetupStep::Provider => {
                "Select LLM provider (↑/↓ to navigate, Enter to select).".to_string()
            }
            SetupStep::ModelName => {
                "Enter model name (e.g. gpt-4o, claude-3-opus).\nPress Enter to confirm. Press Esc to go back."
                    .to_string()
            }
            SetupStep::ApiKey => {
                "Enter API key (leave blank to skip / set later).\nPress Enter to confirm."
                    .to_string()
            }
            SetupStep::MaxIterations => {
                format!(
                    "Max iterations per turn: {}\nPress Enter to confirm.",
                    self.max_iter_input
                )
            }
            SetupStep::Compression => {
                format!(
                    "Compression method: {}\n(summary / token_count / none)\nPress Enter to save.",
                    self.compression_input
                )
            }
            SetupStep::Complete => {
                "✅ Configuration saved!\n\nYou can re-run this wizard with: `oben setup`\nPress q to return to chat."
                    .to_string()
            }
        };
    }

    fn save_config(&mut self) {
        let providers = self.providers();
        let selected_name = if self.provider_selected < providers.len() {
            providers[self.provider_selected]
        } else {
            "OpenRouter"
        };

        self.config.model.kind = match selected_name {
            "OpenRouter" => ProviderKind::OpenRouter,
            "OpenAI" => ProviderKind::OpenAI,
            "Anthropic" => ProviderKind::Anthropic,
            "Bedrock" => ProviderKind::Bedrock,
            "Gemini" => ProviderKind::Gemini,
            "LMStudio (local)" => {
                self.config.model.base_url = Some("http://localhost:1234/v1".to_string());
                ProviderKind::Custom
            }
            _ => ProviderKind::Custom,
        };

        self.config.model.model = self.model_input.clone();
        if !self.api_key_input.is_empty() {
            self.config.model.api_key = Some(self.api_key_input.clone());
        }
        self.config.max_iterations = self.max_iter_input.parse::<usize>().ok();
        self.config.context.compression = self.compression_input.clone();

        if let Err(e) = self.config.save() {
            self.help = format!("Save error: {}", e);
            return;
        }

        let mut tmp = AppConfig::default();
        std::mem::swap(&mut tmp, &mut self.config);
        self.config = tmp;

        self.step = SetupStep::Complete;
    }
}

#[async_trait::async_trait]
impl Panel for SetupPanel {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let para = Paragraph::new(Text::from(self.help.as_str()))
            .block(Block::default().borders(Borders::ALL).title(" Setup "));
        frame.render_widget(para, area);
    }

    async fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                return KeyAction::SwitchPanel(super::PanelId::Chat);
            }
            KeyCode::Esc => {
                self.step = match self.step {
                    SetupStep::Provider => SetupStep::Welcome,
                    SetupStep::ModelName => SetupStep::Provider,
                    SetupStep::ApiKey => SetupStep::ModelName,
                    SetupStep::MaxIterations => SetupStep::ApiKey,
                    SetupStep::Compression => SetupStep::MaxIterations,
                    _ => SetupStep::Welcome,
                };
            }
            KeyCode::Enter => match self.step {
                SetupStep::Welcome
                | SetupStep::Provider
                | SetupStep::ModelName
                | SetupStep::ApiKey
                | SetupStep::MaxIterations => {
                    self.next_step();
                }
                SetupStep::Compression => {
                    self.save_config();
                }
                SetupStep::Complete => {}
            },
            KeyCode::Up | KeyCode::Char('k') if self.step == SetupStep::Provider => {
                if self.provider_selected > 0 {
                    self.provider_selected -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') if self.step == SetupStep::Provider => {
                if self.provider_selected < self.providers().len().saturating_sub(1) {
                    self.provider_selected += 1;
                }
            }
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE => match self.step {
                SetupStep::Compression => match c {
                    '1' => self.compression_input = "summary".to_string(),
                    '2' => self.compression_input = "token_count".to_string(),
                    '3' => self.compression_input = "none".to_string(),
                    _ => {}
                },
                SetupStep::MaxIterations => {
                    if c.is_ascii_digit() {
                        self.max_iter_input.push(c);
                    }
                }
                SetupStep::ModelName => {
                    self.model_input.push(c);
                }
                SetupStep::ApiKey => {
                    self.api_key_input.push(c);
                }
                _ => {}
            },
            KeyCode::Backspace => match self.step {
                SetupStep::MaxIterations => {
                    self.max_iter_input.pop();
                }
                SetupStep::ModelName => {
                    self.model_input.pop();
                }
                SetupStep::ApiKey => {
                    self.api_key_input.pop();
                }
                SetupStep::Compression => {
                    self.compression_input.pop();
                }
                _ => {}
            },
            _ => {}
        }
        KeyAction::None
    }
}
