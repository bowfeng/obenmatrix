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
