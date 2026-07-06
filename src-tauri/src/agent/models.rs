//! Pure parsing of an agent's `list models` stdout into model ids.

/// Parse one model id per non-empty stdout line. Keeps only lines that look
/// like a `provider/model` id (contain a '/'), trimming surrounding whitespace,
/// so banner/help lines are ignored. Deduplicates while preserving order.
pub fn parse_model_ids(stdout: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let id = line.trim();
        if id.contains('/') && !id.contains(' ') && !out.iter().any(|e| e == id) {
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
            parse_model_ids(out),
            vec!["anthropic/claude-opus-4-8".to_string(), "zhipuai-coding-plan/glm-5.2".to_string()]
        );
    }

    #[test]
    fn ignores_banners_blank_and_spaced_lines() {
        let out = "Available models:\n\n  openai/gpt-5  \nnot a model line\n";
        assert_eq!(parse_model_ids(out), vec!["openai/gpt-5".to_string()]);
    }
}
