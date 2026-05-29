//! Frankenstein pipe-syntax pipeline parser (blueprint §10.1).

use crate::error::{AgentHubError, Result};
use thiserror::Error;

/// A single stage in a Frankenstein pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineStage {
    Agent(AgentStage),
    Unix(UnixStage),
}

/// LLM agent stage: optional `@tag` (broadcast when `tag` is `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStage {
    pub tag: Option<String>,
    pub prompt: String,
}

/// Shell command stage (`>` prefix in source syntax).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnixStage {
    pub command: String,
}

/// Parse failure with a byte offset into the original input string.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("pipeline parse error at position {pos}: {msg}")]
pub struct PipelineParseError {
    pub pos: usize,
    pub msg: String,
}

impl From<PipelineParseError> for AgentHubError {
    fn from(e: PipelineParseError) -> Self {
        AgentHubError::PipelineParse {
            pos: e.pos,
            msg: e.msg,
        }
    }
}

type ParseResult<T> = std::result::Result<T, PipelineParseError>;

/// Parse a Frankenstein pipeline string into stages.
///
/// Stages are separated by `" | "` (space-pipe-space). See blueprint §10.1.
pub fn parse(input: &str) -> Result<Vec<PipelineStage>> {
    parse_inner(input).map_err(Into::into)
}

fn parse_inner(input: &str) -> ParseResult<Vec<PipelineStage>> {
    if input.trim().is_empty() {
        return Err(PipelineParseError {
            pos: 0,
            msg: "empty pipeline".into(),
        });
    }

    let segments = split_stages(input);
    let mut stages = Vec::with_capacity(segments.len());

    for (offset, segment) in segments {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            return Err(PipelineParseError {
                pos: offset,
                msg: "empty pipeline stage".into(),
            });
        }

        let stage = if trimmed.starts_with('@') {
            PipelineStage::Agent(parse_agent_stage(trimmed, offset, segment)?)
        } else if trimmed.starts_with('>') {
            PipelineStage::Unix(parse_unix_stage(trimmed)?)
        } else {
            PipelineStage::Agent(AgentStage {
                tag: None,
                prompt: trimmed.to_string(),
            })
        };
        stages.push(stage);
    }

    Ok(stages)
}

/// Split on `" | "` and return `(byte_offset_in_input, segment_slice)`.
fn split_stages(input: &str) -> Vec<(usize, &str)> {
    const DELIM: &str = " | ";
    let mut out = Vec::new();
    let mut start = 0;
    let mut search_from = 0;

    while let Some(rel) = input[search_from..].find(DELIM) {
        let pipe_start = search_from + rel;
        out.push((start, &input[start..pipe_start]));
        start = pipe_start + DELIM.len();
        search_from = start;
    }

    out.push((start, &input[start..]));
    out
}

fn parse_agent_stage(
    trimmed: &str,
    segment_offset: usize,
    raw_segment: &str,
) -> ParseResult<AgentStage> {
    let at_local = trimmed.find('@').ok_or_else(|| PipelineParseError {
        pos: segment_offset,
        msg: "agent stage must start with @".into(),
    })?;
    let after_at = &trimmed[at_local + 1..];

    let tag_len = after_at
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .map(char::len_utf8)
        .sum::<usize>();

    if tag_len == 0 {
        let pos = segment_offset + leading_trim_bytes(raw_segment) + at_local + 1;
        return Err(PipelineParseError {
            pos,
            msg: "expected tag name after @".into(),
        });
    }

    let tag = after_at[..tag_len].to_string();
    let prompt = after_at[tag_len..].trim().to_string();

    Ok(AgentStage {
        tag: Some(tag),
        prompt,
    })
}

fn parse_unix_stage(trimmed: &str) -> ParseResult<UnixStage> {
    let command = trimmed[1..].trim().to_string();
    Ok(UnixStage { command })
}

fn leading_trim_bytes(s: &str) -> usize {
    s.len().saturating_sub(s.trim_start().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(tag: &str, prompt: &str) -> PipelineStage {
        PipelineStage::Agent(AgentStage {
            tag: Some(tag.to_string()),
            prompt: prompt.to_string(),
        })
    }

    fn broadcast(prompt: &str) -> PipelineStage {
        PipelineStage::Agent(AgentStage {
            tag: None,
            prompt: prompt.to_string(),
        })
    }

    fn unix(command: &str) -> PipelineStage {
        PipelineStage::Unix(UnixStage {
            command: command.to_string(),
        })
    }

    #[test]
    fn blueprint_example_three_agent_unix_stages() {
        let input = "@gemini write a Rust HTTP server | > cargo check | @claude fix the errors";
        let stages = parse(input).expect("parse");
        assert_eq!(
            stages,
            vec![
                agent("gemini", "write a Rust HTTP server"),
                unix("cargo check"),
                agent("claude", "fix the errors"),
            ]
        );
    }

    #[test]
    fn blueprint_example_hyphenated_tags_and_echo() {
        let input = r#"@gemini-1 design the schema | @gemini-2 review the schema | > echo "done""#;
        let stages = parse(input).expect("parse");
        assert_eq!(
            stages,
            vec![
                agent("gemini-1", "design the schema"),
                agent("gemini-2", "review the schema"),
                unix(r#"echo "done""#),
            ]
        );
    }

    #[test]
    fn blueprint_example_aider_cargo_claude() {
        let input =
            "@aider implement the login route | > cargo test | @claude summarize test results";
        let stages = parse(input).expect("parse");
        assert_eq!(
            stages,
            vec![
                agent("aider", "implement the login route"),
                unix("cargo test"),
                agent("claude", "summarize test results"),
            ]
        );
    }

    #[test]
    fn broadcast_stage_without_prefix() {
        let input = "kick off the pipeline | > echo ok";
        let stages = parse(input).expect("parse");
        assert_eq!(
            stages,
            vec![broadcast("kick off the pipeline"), unix("echo ok"),]
        );
    }

    #[test]
    fn single_agent_stage() {
        let stages = parse("@solo run").expect("parse");
        assert_eq!(stages, vec![agent("solo", "run")]);
    }

    #[test]
    fn single_unix_stage() {
        let stages = parse("> ls -la").expect("parse");
        assert_eq!(stages, vec![unix("ls -la")]);
    }

    #[test]
    fn agent_with_empty_prompt_is_valid() {
        let stages = parse("@gemini").expect("parse");
        assert_eq!(stages, vec![agent("gemini", "")]);
    }

    #[test]
    fn unix_with_empty_command_is_valid() {
        let stages = parse("> ").expect("parse");
        assert_eq!(stages, vec![unix("")]);
    }

    #[test]
    fn pipe_without_surrounding_spaces_stays_in_segment() {
        let stages = parse("@gemini a|b").expect("parse");
        assert_eq!(stages, vec![agent("gemini", "a|b")]);
    }

    #[test]
    fn error_empty_pipeline() {
        let err = parse("   ").unwrap_err();
        match err {
            AgentHubError::PipelineParse { pos, msg } => {
                assert_eq!(pos, 0);
                assert!(msg.contains("empty pipeline"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn error_empty_stage_between_pipes() {
        let input = "@gemini go |   | > echo";
        let err = parse(input).unwrap_err();
        match err {
            AgentHubError::PipelineParse { pos, msg } => {
                assert_eq!(pos, input.find(" |   | ").unwrap() + 3);
                assert!(msg.contains("empty pipeline stage"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn error_empty_stage_zero_width_between_pipes() {
        let input = "@gemini go |  | > echo";
        let err = parse(input).unwrap_err();
        match err {
            AgentHubError::PipelineParse { pos, msg } => {
                assert_eq!(pos, input.find(" |  | ").unwrap() + 3);
                assert!(msg.contains("empty pipeline stage"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn error_missing_tag_name_position() {
        let input = "@gemini ok | @ | > echo";
        let err = parse(input).unwrap_err();
        match err {
            AgentHubError::PipelineParse { pos, msg } => {
                let at = input.rfind('@').expect("@ stage");
                assert_eq!(pos, at + 1);
                assert!(msg.contains("tag name"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn error_bare_at_sign() {
        let err = parse("@").unwrap_err();
        match err {
            AgentHubError::PipelineParse { pos, msg } => {
                assert_eq!(pos, 1);
                assert!(msg.contains("tag name"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn pipeline_parse_error_converts_to_agent_hub_error() {
        let err = PipelineParseError {
            pos: 7,
            msg: "bad stage".into(),
        };
        let hub: AgentHubError = err.into();
        assert!(matches!(
            hub,
            AgentHubError::PipelineParse {
                pos: 7,
                msg
            } if msg == "bad stage"
        ));
    }
}
