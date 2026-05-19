/// Iteration budget — limits the number of API calls per turn.

use anyhow::{anyhow, Result};

pub struct IterationBudget {
    max_iterations: usize,
    current: usize,
}

impl IterationBudget {
    pub fn new(max_iterations: usize) -> Self {
        Self {
            max_iterations,
            current: 0,
        }
    }

    /// Check if we're still within budget. Returns error if exceeded.
    pub fn check(&mut self) -> Result<()> {
        if self.current >= self.max_iterations {
            return Err(anyhow!(
                "Iteration budget exceeded: {} iterations used (max {}). Agent may be stuck in a loop.",
                self.current,
                self.max_iterations
            ));
        }
        self.current += 1;
        Ok(())
    }

    /// Reset the budget (e.g., after a new turn starts).
    pub fn reset(&mut self) {
        self.current = 0;
    }

    pub fn remaining(&self) -> usize {
        self.max_iterations.saturating_sub(self.current)
    }
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
}
