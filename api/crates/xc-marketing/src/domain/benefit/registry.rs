//! `CalculatorRegistry` (design §6, §7).

use std::collections::HashMap;

use super::calculator::BenefitCalculator;
use super::kinds::fee_waiver::FeeWaiver;
use super::kinds::rate_improvement::RateImprovement;

pub struct CalculatorRegistry {
    calculators: HashMap<String, Box<dyn BenefitCalculator>>,
}

impl Default for CalculatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CalculatorRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            calculators: HashMap::new(),
        };
        registry.register(Box::new(RateImprovement));
        registry.register(Box::new(FeeWaiver));
        registry
    }

    pub fn register(&mut self, calculator: Box<dyn BenefitCalculator>) {
        self.calculators
            .insert(calculator.kind().to_string(), calculator);
    }

    pub fn get(&self, kind: &str) -> Option<&dyn BenefitCalculator> {
        self.calculators.get(kind).map(|b| b.as_ref())
    }
}
