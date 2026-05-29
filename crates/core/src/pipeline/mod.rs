//! Frankenstein pipeline engine (blueprint Part 10).

pub mod parser;

#[cfg(feature = "full")]
pub mod executor;
#[cfg(feature = "full")]
pub mod loop_engine;
#[cfg(feature = "full")]
pub mod pty_bridge;

pub use parser::{
    is_pipeline_input, parse, AgentStage, PipelineParseError, PipelineStage, UnixStage,
};

#[cfg(feature = "full")]
pub use executor::{PipelineExecutor, PipelineResult, AGENT_STAGE_TIMEOUT, UNIX_STAGE_TIMEOUT};
#[cfg(feature = "full")]
pub use loop_engine::{
    parse_spar_command, similarity_ratio, SparConfig, SparEngine, SparResult, DEFAULT_SPAR_TURNS,
    MAX_SPAR_TURNS, SPAR_ABORT, STAGNATION_RATIO,
};
