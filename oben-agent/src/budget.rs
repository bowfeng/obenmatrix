/// Iteration budget — limits the number of API calls per turn with warning thresholds.
///
/// Mirrors Hermes' `_api_call_count` tracking with budget warnings at 80% and 90%.
use anyhow::{anyhow, Result};

/// Callback invoked when approaching iteration budget limits.
pub type BudgetWarningCallback = Box<dyn Fn(usize, usize, u8) + Send + Sync>;

/// Iteration budget with warning support.
pub struct IterationBudget {
    max_iterations: usize,
    current: usize,
    warn_at_80: bool,
    warn_at_90: bool,
    on_warning: Option<BudgetWarningCallback>,
    budget_exhausted_injected: bool,
    budget_grace_call: bool,
    warned_80: bool,
    warned_90: bool,
}

impl IterationBudget {
    pub fn new(max_iterations: usize) -> Self {
        Self {
            max_iterations,
            current: 0,
            warn_at_80: true,
            warn_at_90: true,
            on_warning: None,
            budget_exhausted_injected: false,
            budget_grace_call: false,
            warned_80: false,
            warned_90: false,
        }
    }

    /// Set a callback for budget warnings.
    pub fn on_warning<F>(&mut self, callback: F)
    where
        F: Fn(usize, usize, u8) + Send + Sync + 'static,
    {
        self.on_warning = Some(Box::new(callback));
    }

    /// Check if we're still within budget. Returns error if exceeded.
    ///
    /// Budget enforcement:
    /// - If current >= max_iterations and grace not used: error (exhausted)
    /// - If budget warning was injected before (budget_exhausted_injected=true):
    ///   allow one more grace call, then mark as consumed
    /// - If grace call was already consumed: error
    pub fn check(&mut self) -> Result<()> {
        // If grace call was already consumed, budget is truly exhausted
        if self.budget_grace_call {
            return Err(anyhow!(
                "Iteration budget exhausted: {} API calls made (max {}). Model is still producing tool calls after budget warning.",
                self.current, self.max_iterations
            ));
        }

        // Check if we've hit max
        if self.current >= self.max_iterations {
            if self.budget_exhausted_injected {
                // Budget warning was injected — allow grace call
                self.budget_grace_call = true;
                return Ok(());
            }
            // Budget warning not yet injected — hard error
            return Err(anyhow!(
                "Iteration budget exceeded: {} iterations used (max {}). Agent may be stuck in a loop.",
                self.current, self.max_iterations
            ));
        }

        self.current += 1;
        let pct = self.current * 100 / self.max_iterations.max(1);

        // Warn at 80%
        if self.warn_at_80 && pct >= 80 && !self.warned_80 {
            self.warned_80 = true;
            self.warn(80);
        }

        // Warn at 90%
        if self.warn_at_90 && pct >= 90 && !self.warned_90 {
            self.warned_90 = true;
            self.warn(90);
        }

        Ok(())
    }

    /// Inject the budget warning and allow a grace call.
    /// Call this when the budget warning should be surfaced to the model.
    pub fn inject_warning(&mut self) {
        self.budget_exhausted_injected = true;
    }

    /// Check the budget without incrementing (for pre-flight checks).
    pub fn peek(&self) -> Result<()> {
        if self.budget_grace_call || self.current >= self.max_iterations {
            return Err(anyhow!(
                "Iteration budget exhausted: {} API calls made (max {}).",
                self.current,
                self.max_iterations
            ));
        }
        Ok(())
    }

    pub fn consume_grace_call(&mut self) {
        self.budget_grace_call = true;
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn max(&self) -> usize {
        self.max_iterations
    }

    pub fn remaining(&self) -> usize {
        if self.budget_grace_call {
            return 0;
        }
        self.max_iterations.saturating_sub(self.current)
    }

    pub fn reset(&mut self) {
        self.current = 0;
        self.budget_exhausted_injected = false;
        self.budget_grace_call = false;
        self.warned_80 = false;
        self.warned_90 = false;
    }

    pub fn is_exhausted(&self) -> bool {
        self.budget_exhausted_injected
    }

    pub fn grace_call_used(&self) -> bool {
        self.budget_grace_call
    }

    fn warn(&self, threshold: u8) {
        if let Some(callback) = &self.on_warning {
            callback(self.current, self.max_iterations, threshold);
        }
    }
}

impl Default for IterationBudget {
    fn default() -> Self {
        Self::new(90)
    }
}

/// Create a budget warning message.
pub fn budget_warning_message(threshold: u8) -> String {
    format!(
        "\n⚠️ The agent is approaching its iteration limit ({}% of API calls used). \
         Please provide clear, specific instructions to help the agent complete this task.",
        threshold
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_allows_within_limit() {
        let mut budget = IterationBudget::new(3);
        assert!(budget.check().is_ok());
        assert!(budget.check().is_ok());
        assert!(budget.check().is_ok());
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_budget_rejects_after_limit() {
        let mut budget = IterationBudget::new(2);
        budget.check().unwrap();
        budget.check().unwrap();
        let err = budget.check().unwrap_err();
        assert!(err.to_string().contains("Iteration budget exceeded"));
    }

    #[test]
    fn test_budget_remaining_is_accurate() {
        let mut budget = IterationBudget::new(5);
        assert_eq!(budget.remaining(), 5);
        budget.check().unwrap();
        assert_eq!(budget.remaining(), 4);
        budget.check().unwrap();
        assert_eq!(budget.remaining(), 3);
    }

    #[test]
    fn test_budget_reset_clears_counter() {
        let mut budget = IterationBudget::new(2);
        budget.check().unwrap();
        budget.check().unwrap();
        budget.check().unwrap_err();
        budget.reset();
        assert_eq!(budget.remaining(), 2);
        budget.check().unwrap();
    }

    #[test]
    fn test_budget_zero_max_denies_all() {
        let mut budget = IterationBudget::new(0);
        let err = budget.check().unwrap_err();
        assert!(err.to_string().contains("Iteration budget exceeded"));
    }

    #[test]
    fn test_grace_call_on_exhaustion() {
        let mut budget = IterationBudget::new(2);
        budget.check().unwrap(); // current = 1
        budget.check().unwrap(); // current = 2, hits max
        assert!(budget.is_exhausted() == false); // warning not yet injected

        // Inject warning — this signals budget exhausted
        budget.inject_warning();
        assert!(budget.is_exhausted());
        assert!(!budget.grace_call_used());

        // Grace call: budget_exhausted_injected allows one more check
        budget.consume_grace_call();
        assert!(budget.grace_call_used());

        // Now truly exhausted
        assert!(budget.check().is_err());
    }

    #[test]
    fn test_warning_callback_invoked() {
        let mut budget = IterationBudget::new(10);
        let warnings = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let w = warnings.clone();
        budget.on_warning(move |current, max, threshold| {
            w.lock().unwrap().push((current, max, threshold));
        });

        // Make 8 calls (80%)
        for _ in 0..8 {
            let _ = budget.check();
        }
        let warned = warnings.lock().unwrap();
        assert!(warned.iter().any(|(_, _, t)| *t == 80));
    }

    #[test]
    fn test_budget_warning_message() {
        let msg = budget_warning_message(80);
        assert!(msg.contains("80%"));
        assert!(msg.contains("iteration limit"));
    }

    #[test]
    fn test_peek_does_not_increment() {
        let budget = IterationBudget::new(5);
        assert_eq!(budget.current(), 0);
        budget.peek().unwrap();
        assert_eq!(budget.current(), 0);
        budget.peek().unwrap();
        assert_eq!(budget.current(), 0);
    }
}
