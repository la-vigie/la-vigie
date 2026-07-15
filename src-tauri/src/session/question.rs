//! Pure structured-question core (TASK-122): parse the `AskUserQuestion`
//! `tool_input` a `PreToolUse` hook carries into a `PendingQuestion`, and
//! translate a user's structured selection into the PTY keystrokes that drive
//! the interactive terminal picker. Both halves are pure and unit-tested; the
//! hook capture, `/session` surfacing, and `/answer` PTY writes are glue.

use serde::{Deserialize, Serialize};

/// One selectable option in a question.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// One question in an `AskUserQuestion` call. `prompt` is the question text
/// (Claude's `tool_input` calls it `question`); `multi_select` mirrors
/// `multiSelect`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub prompt: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

/// The full set of questions an agent is currently blocked on, surfaced to the
/// mobile client on the `/session` poll.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestion {
    pub questions: Vec<Question>,
}

/// Parse an `AskUserQuestion` `tool_input` value into questions. Missing fields
/// default (empty strings / no options / single-select) so a malformed payload
/// degrades to an empty/loose card rather than erroring the hook path.
pub fn parse_questions(input: &serde_json::Value) -> Vec<Question> {
    input
        .get("questions")
        .and_then(|q| q.as_array())
        .map(|arr| {
            arr.iter()
                .map(|q| Question {
                    prompt: q.get("question").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    header: q.get("header").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    options: q
                        .get("options")
                        .and_then(|o| o.as_array())
                        .map(|opts| {
                            opts.iter()
                                .map(|o| QuestionOption {
                                    label: o.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                    description: o
                                        .get("description")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    multi_select: q.get("multiSelect").and_then(|v| v.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A user's answer to one question. `Options` carries the chosen option
/// index(es) (one for single-select, many for multi-select); `Custom` is the
/// free-text "Other" path. Untagged: a body with `optionIndices` deserializes
/// to `Options`, one with `custom` to `Custom`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Answer {
    Options {
        #[serde(rename = "optionIndices")]
        option_indices: Vec<usize>,
    },
    Custom {
        custom: String,
    },
}

// ── Pre-spike picker keystroke model (TASK-122) ──────────────────────────────
// The exact bytes the AskUserQuestion TUI picker consumes are NOT documented and
// MUST be confirmed by the live PTY spike (plan Task 0) before shipping — they
// are centralized here so the spike adjusts one place. Current guess: the picker
// starts on the first option; ↓ moves down one; Enter submits/advances to the
// next question; Space toggles a multi-select option; the free-text "Other"
// entry sits one row past the last option and is focused with Enter.
const KEY_DOWN: &str = "\x1b[B";
const KEY_ENTER: &str = "\r";
const KEY_SPACE: &str = " ";

/// Translate a full set of answers (one per question, in order) into the ordered
/// PTY write chunks that drive the picker. The caller writes each chunk as a
/// separate PTY write with a short delay between (so Enter registers as a
/// distinct read, per the `/reply` lesson). Errors (→ HTTP 400) on an arity
/// mismatch, an out-of-range index, an empty selection, or multiple indices for
/// a single-select question.
pub fn questions_to_keystrokes(questions: &[Question], answers: &[Answer]) -> Result<Vec<String>, String> {
    if questions.len() != answers.len() {
        return Err(format!("expected {} answers, got {}", questions.len(), answers.len()));
    }
    let mut chunks = Vec::new();
    for (qi, (q, a)) in questions.iter().zip(answers).enumerate() {
        match a {
            Answer::Options { option_indices } => {
                if option_indices.is_empty() {
                    return Err(format!("question {qi}: no option selected"));
                }
                for &idx in option_indices {
                    if idx >= q.options.len() {
                        return Err(format!("question {qi}: option index {idx} out of range"));
                    }
                }
                if !q.multi_select {
                    if option_indices.len() != 1 {
                        return Err(format!("question {qi}: single-select expects exactly one option"));
                    }
                    let idx = option_indices[0];
                    if idx > 0 {
                        chunks.push(KEY_DOWN.repeat(idx));
                    }
                    chunks.push(KEY_ENTER.to_string());
                } else {
                    let mut sel = option_indices.clone();
                    sel.sort_unstable();
                    sel.dedup();
                    let max = *sel.last().unwrap();
                    let mut nav = String::new();
                    for pos in 0..=max {
                        if sel.contains(&pos) {
                            nav.push_str(KEY_SPACE);
                        }
                        if pos < max {
                            nav.push_str(KEY_DOWN);
                        }
                    }
                    chunks.push(nav);
                    chunks.push(KEY_ENTER.to_string());
                }
            }
            Answer::Custom { custom } => {
                // Navigate one row past the last real option to the "Other" entry,
                // Enter to focus the text field, type the text, Enter to submit.
                if !q.options.is_empty() {
                    chunks.push(KEY_DOWN.repeat(q.options.len()));
                }
                chunks.push(KEY_ENTER.to_string());
                chunks.push(custom.clone());
                chunks.push(KEY_ENTER.to_string());
            }
        }
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASK: &[u8] = include_bytes!("fixtures/ask_user_question.json");

    #[test]
    fn parses_questions_from_tool_input() {
        let v: serde_json::Value = serde_json::from_slice(ASK).unwrap();
        let qs = parse_questions(&v);
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].prompt, "How should I format the output?");
        assert_eq!(qs[0].header, "Format");
        assert!(!qs[0].multi_select);
        assert_eq!(qs[0].options.len(), 2);
        assert_eq!(qs[0].options[0].label, "Summary");
        assert_eq!(qs[0].options[0].description, "Brief overview");
        assert!(qs[1].multi_select);
    }

    #[test]
    fn missing_questions_key_yields_empty() {
        assert!(parse_questions(&serde_json::json!({})).is_empty());
    }

    fn q(multi: bool, n_opts: usize) -> Question {
        Question {
            prompt: "P".into(),
            header: "H".into(),
            options: (0..n_opts)
                .map(|i| QuestionOption { label: format!("O{i}"), description: String::new() })
                .collect(),
            multi_select: multi,
        }
    }

    #[test]
    fn single_select_navigates_by_arrow_then_enter() {
        let qs = [q(false, 3)];
        let ans = [Answer::Options { option_indices: vec![1] }];
        assert_eq!(questions_to_keystrokes(&qs, &ans).unwrap(), vec!["\x1b[B".to_string(), "\r".to_string()]);
    }

    #[test]
    fn single_select_index_zero_is_just_enter() {
        let qs = [q(false, 3)];
        let ans = [Answer::Options { option_indices: vec![0] }];
        assert_eq!(questions_to_keystrokes(&qs, &ans).unwrap(), vec!["\r".to_string()]);
    }

    #[test]
    fn single_select_rejects_multiple_indices() {
        let qs = [q(false, 3)];
        let ans = [Answer::Options { option_indices: vec![0, 1] }];
        assert!(questions_to_keystrokes(&qs, &ans).is_err());
    }

    #[test]
    fn multi_select_toggles_each_then_enter() {
        let qs = [q(true, 3)];
        let ans = [Answer::Options { option_indices: vec![0, 2] }];
        // pos0 Space, Down, (pos1 nothing) Down, pos2 Space, then Enter.
        assert_eq!(
            questions_to_keystrokes(&qs, &ans).unwrap(),
            vec![" \x1b[B\x1b[B ".to_string(), "\r".to_string()]
        );
    }

    #[test]
    fn custom_navigates_past_options_then_types() {
        let qs = [q(false, 2)];
        let ans = [Answer::Custom { custom: "hello".into() }];
        assert_eq!(
            questions_to_keystrokes(&qs, &ans).unwrap(),
            vec!["\x1b[B\x1b[B".to_string(), "\r".to_string(), "hello".to_string(), "\r".to_string()]
        );
    }

    #[test]
    fn arity_mismatch_and_out_of_range_error() {
        assert!(questions_to_keystrokes(&[q(false, 2)], &[]).is_err());
        let ans = [Answer::Options { option_indices: vec![9] }];
        assert!(questions_to_keystrokes(&[q(false, 2)], &ans).is_err());
    }

    #[test]
    fn two_questions_concatenate_in_order() {
        let qs = [q(false, 2), q(false, 2)];
        let ans = [
            Answer::Options { option_indices: vec![0] },
            Answer::Options { option_indices: vec![1] },
        ];
        assert_eq!(
            questions_to_keystrokes(&qs, &ans).unwrap(),
            vec!["\r".to_string(), "\x1b[B".to_string(), "\r".to_string()]
        );
    }

    #[test]
    fn answer_deserializes_untagged() {
        let opt: Answer = serde_json::from_str(r#"{"optionIndices":[1]}"#).unwrap();
        assert_eq!(opt, Answer::Options { option_indices: vec![1] });
        let custom: Answer = serde_json::from_str(r#"{"custom":"hi"}"#).unwrap();
        assert_eq!(custom, Answer::Custom { custom: "hi".into() });
    }
}
