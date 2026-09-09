//! Microsecond Declarative Capability Policy Evaluation Engine.
//!
//! Provides deterministic fixed-point evaluation of first-order authorization policy rules
//! for CelNet institutional nodes and execution clusters.

use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};

/// Term in a policy predicate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Term {
    /// String symbol or identifier.
    String(String),
    /// 64-bit signed integer.
    Integer(i64),
    /// Boolean flag.
    Boolean(bool),
    /// Variable identifier (e.g. "$x", "$cores").
    Variable(String),
}

impl Term {
    /// Check if this term is a variable.
    pub fn is_variable(&self) -> bool {
        matches!(self, Term::Variable(_))
    }
}

/// A ground or patterned policy fact (predicate).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Fact {
    /// Predicate name (e.g. "licensed_asset", "max_cores").
    pub predicate: String,
    /// Ordered arguments.
    pub terms: Vec<Term>,
}

impl Fact {
    /// Construct a new fact.
    pub fn new(predicate: impl Into<String>, terms: Vec<Term>) -> Self {
        Self {
            predicate: predicate.into(),
            terms,
        }
    }

    /// Check if all terms are ground (no variables).
    pub fn is_ground(&self) -> bool {
        self.terms.iter().all(|t| !t.is_variable())
    }
}

/// Comparison operators for variable constraints.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Op {
    /// Less than (<)
    Lt,
    /// Less than or equal (<=)
    Le,
    /// Greater than (>)
    Gt,
    /// Greater than or equal (>=)
    Ge,
    /// Equal (==)
    Eq,
    /// Not equal (!=)
    Ne,
}

/// Binary constraint applied to a variable in a rule body.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Constraint {
    /// Variable name (e.g. "$cores").
    pub variable: String,
    /// Comparison operator.
    pub op: Op,
    /// Ground value compared against.
    pub target: Term,
}

impl Constraint {
    /// Evaluate the constraint against a variable binding.
    pub fn evaluate(&self, bindings: &HashMap<String, Term>) -> bool {
        let Some(val) = bindings.get(&self.variable) else {
            return false;
        };
        match (val, &self.target) {
            (Term::Integer(a), Term::Integer(b)) => match self.op {
                Op::Lt => a < b,
                Op::Le => a <= b,
                Op::Gt => a > b,
                Op::Ge => a >= b,
                Op::Eq => a == b,
                Op::Ne => a != b,
            },
            (Term::String(a), Term::String(b)) => match self.op {
                Op::Eq => a == b,
                Op::Ne => a != b,
                Op::Lt => a < b,
                Op::Le => a <= b,
                Op::Gt => a > b,
                Op::Ge => a >= b,
            },
            (Term::Boolean(a), Term::Boolean(b)) => match self.op {
                Op::Eq => a == b,
                Op::Ne => a != b,
                _ => false,
            },
            _ => false,
        }
    }
}

/// An authorization policy rule: Head <- Body[0], Body[1], ..., Constraints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Consequent fact generated when body matches.
    pub head: Fact,
    /// Antecedent patterns to match against known facts.
    pub body: Vec<Fact>,
    /// Value constraints on bound variables.
    pub constraints: Vec<Constraint>,
}

/// An authorization check caveat: `check if Q1, Q2...`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// Queries that must yield at least one satisfying ground fact.
    pub queries: Vec<Rule>,
}

/// The evaluation state of the institutional policy knowledge base.
#[derive(Debug, Default, Clone)]
pub struct DatalogEngine {
    facts: HashSet<Fact>,
    rules: Vec<Rule>,
}

/// Institutional capability policy evaluation engine alias.
pub type PolicyEngine = DatalogEngine;
/// Policy ground fact predicate.
pub type PolicyFact = Fact;
/// Policy inference rule.
pub type PolicyRule = Rule;
/// Policy authorization caveat check.
pub type PolicyCheck = Check;
/// Policy variable binary constraint.
pub type PolicyConstraint = Constraint;
/// Policy comparison operator.
pub type PolicyOp = Op;
/// Policy predicate term.
pub type PolicyTerm = Term;

impl DatalogEngine {
    /// Create a new empty policy engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a ground fact to the knowledge base.
    pub fn add_fact(&mut self, fact: Fact) {
        if fact.is_ground() {
            self.facts.insert(fact);
        }
    }

    /// Add a rule to the knowledge base.
    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Execute fixed-point iteration to derive all reachable facts.
    pub fn run_fixed_point(&mut self, max_iterations: usize) {
        for _ in 0..max_iterations {
            let mut derived = HashSet::new();
            for rule in &self.rules {
                let mut matches = vec![HashMap::new()];
                for pattern in &rule.body {
                    let mut next_matches = Vec::new();
                    for m in &matches {
                        for fact in &self.facts {
                            if let Some(extended) = unify_fact(pattern, fact, m) {
                                next_matches.push(extended);
                            }
                        }
                    }
                    matches = next_matches;
                    if matches.is_empty() {
                        break;
                    }
                }

                for m in matches {
                    if !rule.constraints.iter().all(|c| c.evaluate(&m)) {
                        continue;
                    }
                    if let Some(head_fact) =
                        instantiate_fact(&rule.head, &m).filter(|f| !self.facts.contains(f))
                    {
                        derived.insert(head_fact);
                    }
                }
            }

            if derived.is_empty() {
                break;
            }
            self.facts.extend(derived);
        }
    }

    /// Check if a ground fact exists in the evaluated state.
    pub fn contains_fact(&self, fact: &Fact) -> bool {
        self.facts.contains(fact)
    }

    /// Query all facts matching a given predicate name.
    pub fn query_predicate(&self, predicate: &str) -> Vec<Fact> {
        self.facts
            .iter()
            .filter(|f| f.predicate == predicate)
            .cloned()
            .collect()
    }

    /// Evaluate an authorization check against the current fact base.
    pub fn verify_check(&self, check: &Check) -> Result<(), String> {
        for query in &check.queries {
            let mut matches = vec![HashMap::new()];
            for pattern in &query.body {
                let mut next_matches = Vec::new();
                for m in &matches {
                    for fact in &self.facts {
                        if let Some(extended) = unify_fact(pattern, fact, m) {
                            next_matches.push(extended);
                        }
                    }
                }
                matches = next_matches;
                if matches.is_empty() {
                    break;
                }
            }

            let any_valid = matches.iter().any(|m| {
                query.constraints.iter().all(|c| c.evaluate(m))
            });

            if any_valid {
                return Ok(());
            }
        }
        Err("check constraints not satisfied by active facts".to_string())
    }
}

fn unify_fact(pattern: &Fact, fact: &Fact, current_bindings: &HashMap<String, Term>) -> Option<HashMap<String, Term>> {
    if pattern.predicate != fact.predicate || pattern.terms.len() != fact.terms.len() {
        return None;
    }

    let mut bindings = current_bindings.clone();
    for (p, f) in pattern.terms.iter().zip(fact.terms.iter()) {
        match p {
            Term::Variable(v) => {
                if let Some(existing) = bindings.get(v) {
                    if existing != f {
                        return None;
                    }
                } else {
                    bindings.insert(v.clone(), f.clone());
                }
            }
            ground => {
                if ground != f {
                    return None;
                }
            }
        }
    }
    Some(bindings)
}

fn instantiate_fact(template: &Fact, bindings: &HashMap<String, Term>) -> Option<Fact> {
    let mut ground_terms = Vec::with_capacity(template.terms.len());
    for t in &template.terms {
        match t {
            Term::Variable(v) => {
                let bound = bindings.get(v)?;
                ground_terms.push(bound.clone());
            }
            ground => ground_terms.push(ground.clone()),
        }
    }
    Some(Fact {
        predicate: template.predicate.clone(),
        terms: ground_terms,
    })
}
