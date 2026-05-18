/// Judge module — evaluates whether a goal is satisfied.
///
/// The judge is an auxiliary LLM call that receives the goal and the agent's
/// last response, then outputs a verdict: DONE or CONTINUE.

pub mod verdict;

pub use verdict::{JudgeVerdict, parse_judge_response};
