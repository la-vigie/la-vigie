//! Pure parsing of an agent's `list models` stdout into model ids.

/// Parse one model id per non-empty stdout line, trimming surrounding
/// whitespace and deduplicating while preserving order.
///
/// `provider_slash_ids_only` selects the output shape to expect, since
/// different CLIs print genuinely different things and no punctuation-based
/// heuristic reliably tells them apart without risking false positives
/// (admitting a banner line) or false negatives (silently dropping a real
/// model line):
/// - `true` — only lines that look like a `provider/model` id (contain a
///   '/' and no spaces) are kept, e.g. opencode's `openai/gpt-5`; banner/help
///   lines (which opencode's `models` output can include) are ignored.
/// - `false` — every non-blank line is kept verbatim, for CLIs whose
///   enumeration output is just one plain model name per line with nothing
///   else to filter out, e.g. antigravity's `agy models` (see
///   `fixtures/agy_models_v1.1.4.txt`, captured from the real binary).
pub fn parse_model_ids(stdout: &str, provider_slash_ids_only: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let id = line.trim();
        if id.is_empty() || out.iter().any(|e| e == id) {
            continue;
        }
        let keep = !provider_slash_ids_only || (id.contains('/') && !id.contains(' '));
        if keep {
            out.push(id.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_provider_slash_model_lines_in_order_dedup() {
        let out = "anthropic/claude-opus-4-8\nzhipuai-coding-plan/glm-5.2\nzhipuai-coding-plan/glm-5.2\n";
        assert_eq!(
            parse_model_ids(out, true),
            vec!["anthropic/claude-opus-4-8".to_string(), "zhipuai-coding-plan/glm-5.2".to_string()]
        );
    }

    #[test]
    fn ignores_banners_blank_and_spaced_lines() {
        let out = "Available models:\n\n  openai/gpt-5  \nnot a model line\n";
        assert_eq!(parse_model_ids(out, true), vec!["openai/gpt-5".to_string()]);
    }

    #[test]
    fn ignores_banner_lines_with_an_embedded_slash_or_url() {
        let out = "Available models (see docs at https://opencode.ai/models):\nanthropic/claude-opus-4-5\n";
        assert_eq!(parse_model_ids(out, true), vec!["anthropic/claude-opus-4-5".to_string()]);
    }

    #[test]
    fn plain_label_mode_keeps_every_non_blank_line_verbatim_no_filtering() {
        // Deliberate: unlike the strict provider/slash mode, plain-label mode
        // does no banner filtering at all — it's only safe to use for a CLI
        // whose `models` output is confirmed to never emit banner/help lines
        // (see `keeps_display_labels_from_the_real_antigravity_models_fixture`).
        let out = "Usage: agy models (lists all available models)\nGemini 3.1 Pro (High)\n";
        assert_eq!(
            parse_model_ids(out, false),
            vec!["Usage: agy models (lists all available models)".to_string(), "Gemini 3.1 Pro (High)".to_string()]
        );
    }

    #[test]
    fn keeps_display_labels_from_the_real_antigravity_models_fixture() {
        let out = include_str!("fixtures/agy_models_v1.1.4.txt");
        assert_eq!(
            parse_model_ids(out, false),
            vec![
                "Gemini 3.5 Flash (Medium)".to_string(),
                "Gemini 3.5 Flash (High)".to_string(),
                "Gemini 3.5 Flash (Low)".to_string(),
                "Gemini 3.1 Pro (Low)".to_string(),
                "Gemini 3.1 Pro (High)".to_string(),
                "Claude Sonnet 4.6 (Thinking)".to_string(),
                "Claude Opus 4.6 (Thinking)".to_string(),
                "GPT-OSS 120B (Medium)".to_string(),
            ]
        );
    }
}
