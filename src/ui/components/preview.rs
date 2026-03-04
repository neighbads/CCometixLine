use crate::config::{Config, SegmentId};
use crate::core::segments::SegmentData;
use crate::core::StatusLineGenerator;
use ratatui::{
    layout::Rect,
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::collections::HashMap;

pub struct PreviewComponent {
    preview_cache: String,
    preview_text: Text<'static>,
    custom_values: HashMap<String, String>,
}

impl Default for PreviewComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewComponent {
    pub fn new() -> Self {
        Self {
            preview_cache: String::new(),
            preview_text: Text::default(),
            custom_values: HashMap::new(),
        }
    }

    pub fn update_preview(&mut self, config: &Config) {
        self.update_preview_with_width(config, 80); // Default width
    }

    pub fn update_preview_with_width(&mut self, config: &Config, width: u16) {
        // Generate mock segments data directly for preview
        let segments_data = self.generate_mock_segments_data(config);

        // Generate both string and TUI text versions
        let renderer = StatusLineGenerator::new(config.clone());

        // Keep string version for compatibility (if needed elsewhere)
        self.preview_cache = renderer.generate(segments_data.clone());

        // Generate TUI-optimized text with smart segment wrapping for preview display
        // Use actual available width minus borders
        let content_width = width.saturating_sub(2);
        let preview_result = renderer.generate_for_tui_preview(segments_data, content_width);

        // Convert to owned text by cloning the spans
        let owned_lines: Vec<Line<'static>> = preview_result
            .lines
            .into_iter()
            .map(|line| {
                let owned_spans: Vec<ratatui::text::Span<'static>> = line
                    .spans
                    .into_iter()
                    .map(|span| ratatui::text::Span::styled(span.content.to_string(), span.style))
                    .collect();
                Line::from(owned_spans)
            })
            .collect();

        self.preview_text = Text::from(owned_lines);
    }

    pub fn calculate_height(&self) -> u16 {
        let line_count = self.preview_text.lines.len().max(1);
        // Min 3 (1 line + 2 borders), max 8 to prevent taking too much space
        ((line_count + 2).max(3) as u16).min(8)
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        let preview = Paragraph::new(self.preview_text.clone())
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .wrap(ratatui::widgets::Wrap { trim: false });
        f.render_widget(preview, area);
    }

    pub fn get_preview_cache(&self) -> &str {
        &self.preview_cache
    }

    pub fn flush_custom(&mut self, config: &Config) {
        use crate::utils::logger::log_debug;

        self.custom_values.clear();
        let custom_count = config
            .segments
            .iter()
            .filter(|s| matches!(s.id, SegmentId::Custom(_)))
            .count();
        log_debug(
            "custom:flush",
            &format!("flushing {} custom segments", custom_count),
        );

        for segment_config in &config.segments {
            if let SegmentId::Custom(ref name) = segment_config.id {
                if !segment_config.enabled {
                    log_debug(
                        "custom:flush",
                        &format!("segment '{}' skipped: disabled", name),
                    );
                    continue;
                }
                let command = segment_config
                    .options
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if command.is_empty() {
                    log_debug(
                        "custom:flush",
                        &format!("segment '{}' skipped: empty command", name),
                    );
                    continue;
                }
                log_debug(
                    "custom:flush",
                    &format!("segment '{}' executing: {}", name, command),
                );
                let output = if cfg!(target_os = "windows") {
                    std::process::Command::new("cmd")
                        .args(["/C", command])
                        .output()
                } else {
                    std::process::Command::new("sh")
                        .args(["-c", command])
                        .output()
                };
                match output {
                    Ok(output) if output.status.success() => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let value = stdout.lines().next().unwrap_or("").trim().to_string();
                        if !value.is_empty() {
                            log_debug(
                                "custom:flush",
                                &format!("segment '{}' result: {}", name, value),
                            );
                            self.custom_values.insert(name.clone(), value);
                        } else {
                            log_debug(
                                "custom:flush",
                                &format!("segment '{}' returned empty output", name),
                            );
                        }
                    }
                    Ok(output) => {
                        log_debug(
                            "custom:flush",
                            &format!("segment '{}' exited with status: {}", name, output.status),
                        );
                    }
                    Err(e) => {
                        log_debug(
                            "custom:flush",
                            &format!("segment '{}' command failed: {}", name, e),
                        );
                    }
                }
            }
        }
    }

    /// Generate mock segments data for preview display
    /// This creates perfect preview data without depending on real environment
    fn generate_mock_segments_data(
        &self,
        config: &Config,
    ) -> Vec<(crate::config::SegmentConfig, SegmentData)> {
        let mut segments_data = Vec::new();

        for segment_config in &config.segments {
            if !segment_config.enabled {
                continue;
            }

            let mock_data = match segment_config.id {
                SegmentId::Model => SegmentData {
                    primary: "Sonnet 4".to_string(),
                    secondary: "".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert("model".to_string(), "claude-4-sonnet-20250512".to_string());
                        map
                    },
                },
                SegmentId::Directory => SegmentData {
                    primary: "CCometixLine".to_string(),
                    secondary: "".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert("current_dir".to_string(), "~/CCometixLine".to_string());
                        map
                    },
                },
                SegmentId::Git => SegmentData {
                    primary: "master".to_string(),
                    secondary: "✓".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert("branch".to_string(), "master".to_string());
                        map.insert("status".to_string(), "Clean".to_string());
                        map.insert("ahead".to_string(), "0".to_string());
                        map.insert("behind".to_string(), "0".to_string());
                        map
                    },
                },
                SegmentId::ContextWindow => SegmentData {
                    primary: "78.2%".to_string(),
                    secondary: "· 156.4k".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert("total_tokens".to_string(), "156400".to_string());
                        map.insert("percentage".to_string(), "78.2".to_string());
                        map.insert("session_tokens".to_string(), "48200".to_string());
                        map
                    },
                },
                SegmentId::Usage => SegmentData {
                    primary: "24%".to_string(),
                    secondary: "· 10.7 02:00".to_string(),
                    metadata: HashMap::new(),
                },
                SegmentId::Cost => SegmentData {
                    primary: "$0.02".to_string(),
                    secondary: "".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert("cost".to_string(), "0.01234".to_string());
                        map
                    },
                },
                SegmentId::Session => SegmentData {
                    primary: "3m45s".to_string(),
                    secondary: "\x1b[32m+156\x1b[0m \x1b[31m-23\x1b[0m".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert("duration_ms".to_string(), "225000".to_string());
                        map.insert("lines_added".to_string(), "156".to_string());
                        map.insert("lines_removed".to_string(), "23".to_string());
                        map
                    },
                },
                SegmentId::OutputStyle => SegmentData {
                    primary: "default".to_string(),
                    secondary: "".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert("style_name".to_string(), "default".to_string());
                        map
                    },
                },
                SegmentId::Update => SegmentData {
                    primary: format!("v{}", env!("CARGO_PKG_VERSION")),
                    secondary: "".to_string(),
                    metadata: {
                        let mut map = HashMap::new();
                        map.insert(
                            "current_version".to_string(),
                            env!("CARGO_PKG_VERSION").to_string(),
                        );
                        map.insert("update_available".to_string(), "false".to_string());
                        map
                    },
                },
                SegmentId::Custom(ref name) => {
                    let value = self
                        .custom_values
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| "[no data]".to_string());
                    SegmentData {
                        primary: value,
                        secondary: "".to_string(),
                        metadata: {
                            let mut map = HashMap::new();
                            map.insert("custom_name".to_string(), name.clone());
                            map
                        },
                    }
                }
            };

            segments_data.push((segment_config.clone(), mock_data));
        }

        segments_data
    }
}
