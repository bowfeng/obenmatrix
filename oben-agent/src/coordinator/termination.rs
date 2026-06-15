/// Pluggable termination policies for the conversation coordinator.
///
/// Each policy evaluates whether the loop should stop. The coordinator checks
/// all registered policies after each turn. If ANY policy returns `Some(reason)`,
/// the loop exits.
///
/// Use `TerminationPolicyGroup` for combining multiple policies with custom logic.
use anyhow::Result;

/// What caused the loop to terminate.
#[derive(Debug, Clone, PartialEq)]
pub enum ExitReason {
    /// User explicitly chose to exit.
    UserQuit,
    /// Max turns/iterations reached.
    BudgetExhausted,
    /// External interrupt (Ctrl+C, platform disconnect).
    Interrupted,
    /// External time limit exceeded.
    Timeout,
    /// LLM determined conversation is complete.
    LlmDecidedDone,
    /// Other reason (custom).
    Other(String),
}

/// A policy that evaluates whether conversation should terminate.
///
/// Each implementation focuses on ONE termination criterion. The coordinator
/// evaluates all registered policies and exits if ANY policy requests exit.
pub trait TerminationPolicy: Send + Sync {
    /// Evaluate whether the conversation should terminate.
    ///
    /// Returns `Ok(Some(ExitReason))` if termination is requested,
    /// `Ok(None)` if the loop should continue, or `Err` for policy errors.
    fn evaluate(
        &self,
        turn_count: usize,
        msg_count: usize,
        last_response: &str,
        last_error: Option<&anyhow::Error>,
    ) -> Result<Option<ExitReason>>;
}

/// Budget-based termination — stops after max_turns.
///
/// Mirrors the current `max_iterations` behavior from AppConfig.
pub struct BudgetTerminationPolicy {
    pub max_turns: usize,
}

impl BudgetTerminationPolicy {
    pub fn new(max_turns: usize) -> Self {
        Self { max_turns }
    }
}

impl Default for BudgetTerminationPolicy {
    fn default() -> Self {
        Self::new(50)
    }
}

impl TerminationPolicy for BudgetTerminationPolicy {
    fn evaluate(
        &self,
        turn_count: usize,
        _msg_count: usize,
        _last_response: &str,
        _last_error: Option<&anyhow::Error>,
    ) -> Result<Option<ExitReason>> {
        if turn_count >= self.max_turns {
            Ok(Some(ExitReason::BudgetExhausted))
        } else {
            Ok(None)
        }
    }
}

/// Time-based termination — stops after max_duration.
pub struct TimeTerminationPolicy {
    pub max_duration: std::time::Duration,
    start: std::time::Instant,
}

impl TimeTerminationPolicy {
    pub fn new(max_duration: std::time::Duration) -> Self {
        Self {
            max_duration,
            start: std::time::Instant::now(),
        }
    }
}

impl TerminationPolicy for TimeTerminationPolicy {
    fn evaluate(
        &self,
        _turn_count: usize,
        _msg_count: usize,
        _last_response: &str,
        _last_error: Option<&anyhow::Error>,
    ) -> Result<Option<ExitReason>> {
        if self.start.elapsed() >= self.max_duration {
            Ok(Some(ExitReason::Timeout))
        } else {
            Ok(None)
        }
    }
}

/// Group of policies. Exits on the first matching policy.
pub struct TerminationPolicyGroup {
    policies: Vec<Box<dyn TerminationPolicy>>,
}

impl TerminationPolicyGroup {
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
        }
    }

    pub fn with_policy(mut self, policy: Box<dyn TerminationPolicy>) -> Self {
        self.policies.push(policy);
        self
    }

    pub fn add_policy(&mut self, policy: Box<dyn TerminationPolicy>) {
        self.policies.push(policy);
    }
}

impl Default for TerminationPolicyGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminationPolicy for TerminationPolicyGroup {
    fn evaluate(
        &self,
        turn_count: usize,
        msg_count: usize,
        last_response: &str,
        last_error: Option<&anyhow::Error>,
    ) -> Result<Option<ExitReason>> {
        for policy in &self.policies {
            if let Some(reason) = policy.evaluate(turn_count, msg_count, last_response, last_error)? {
                return Ok(Some(reason));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_policy_allows_within_limit() {
        let policy = BudgetTerminationPolicy::new(5);
        for i in 0..5 {
            assert!(policy.evaluate(i, 0, "", None).unwrap().is_none());
        }
    }

    #[test]
    fn test_budget_policy_exits_at_limit() {
        let policy = BudgetTerminationPolicy::new(3);
        // Turn 3 exceeds max_turns=3
        let result = policy.evaluate(3, 0, "", None).unwrap().unwrap();
        assert_eq!(result, ExitReason::BudgetExhausted);
    }

    #[test]
    fn test_policy_group_fires_first_matching() {
        let group = TerminationPolicyGroup::new()
            .with_policy(Box::new(BudgetTerminationPolicy::new(100)))
            .with_policy(Box::new(TimeTerminationPolicy::new(
                std::time::Duration::from_millis(0),
            )));

        // Time policy fires (instant timeout)
        let result = group.evaluate(0, 0, "", None).unwrap().unwrap();
        assert_eq!(result, ExitReason::Timeout);
    }
}
