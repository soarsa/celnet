//! The routable fields, operators, and values — the typed vocabulary a routing
//! rule is built from, plus the comparison semantics.
//!
//! Every rule is `IF <field> <op> <value> THEN …`. Three enums pin that down:
//! [`RouteField`] (which trade attribute), [`RouteOp`] (the comparison), and
//! [`RouteValue`] (the literal). A [`FieldKind`] classifies each field so the
//! server (and the GUI editor) can restrict which operators and value shapes are
//! legal — a malformed rule such as `side > 5` is rejected by validation and, in
//! the UI, unrepresentable. This module owns that type matrix and the total,
//! never-panicking [`RouteOp::eval`].

use serde::{Deserialize, Serialize};

/// The type class of a routable field. Drives which [`RouteOp`]s are valid and
/// which [`RouteValue`] shape a condition must carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    /// A value drawn from a fixed or registry-backed set (e.g. `side`,
    /// `product`, `ccy`, `desk`). Compared by string equality / membership;
    /// substring (`Contains`) is deliberately not offered.
    Enum,
    /// A real-valued quantity (e.g. `notional`, `strike`). Supports ordering and
    /// range operators.
    Numeric,
    /// Free-form text (e.g. `instrument_id`). Supports equality, membership, and
    /// substring.
    String,
}

/// A trade field a routing rule can match against. The one-to-one image of
/// [`crate::RoutingContext`]'s fields. New routable fields are added by
/// extending both in lock-step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RouteField {
    /// Instrument identifier / symbol (string).
    InstrumentId,
    /// Currency or pair (enum).
    Ccy,
    /// Product family — vanilla / swap / bond / … (enum).
    Product,
    /// Trade side — Buy / Sell (enum).
    Side,
    /// Absolute base notional of the fill (numeric).
    Notional,
    /// Tenor / expiry in years (numeric).
    Tenor,
    /// Strike / resolved level (numeric).
    Strike,
    /// Counterparty / originating FIX session (enum).
    Counterparty,
    /// Booking user (enum).
    User,
    /// Owning desk (enum).
    Desk,
    /// Fill price / premium (numeric).
    Price,
}

impl RouteField {
    /// The [`FieldKind`] of this field — the single source of truth for the
    /// operator/value type matrix used by [`RouteOp::valid_for`] and validation.
    pub fn kind(self) -> FieldKind {
        match self {
            RouteField::Side
            | RouteField::Product
            | RouteField::Ccy
            | RouteField::Counterparty
            | RouteField::User
            | RouteField::Desk => FieldKind::Enum,
            RouteField::Notional | RouteField::Tenor | RouteField::Strike | RouteField::Price => {
                FieldKind::Numeric
            }
            RouteField::InstrumentId => FieldKind::String,
        }
    }
}

/// A comparison operator in a routing condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Strictly greater than (numeric only).
    Gt,
    /// Greater than or equal (numeric only).
    Ge,
    /// Strictly less than (numeric only).
    Lt,
    /// Less than or equal (numeric only).
    Le,
    /// Substring containment (string only).
    Contains,
    /// Membership in a [`RouteValue::List`].
    In,
    /// Inclusive range `lo ≤ x ≤ hi` (numeric only), against a
    /// [`RouteValue::Range`].
    Between,
}

impl RouteOp {
    /// Whether this operator is valid for a field of the given [`FieldKind`].
    ///
    /// The matrix (see [`crate`] docs):
    /// - **Numeric** → `Eq Ne Gt Ge Lt Le Between`
    /// - **String**  → `Eq Ne Contains In`
    /// - **Enum**    → `Eq Ne In` (no `Contains` — an enum has no substring)
    pub fn valid_for(self, kind: FieldKind) -> bool {
        use RouteOp::*;
        match kind {
            FieldKind::Numeric => matches!(self, Eq | Ne | Gt | Ge | Lt | Le | Between),
            FieldKind::String => matches!(self, Eq | Ne | Contains | In),
            FieldKind::Enum => matches!(self, Eq | Ne | In),
        }
    }

    /// Evaluate `ctx <op> val`, totally — **any** type-mismatched combination of
    /// `(op, ctx, val)` returns `false` and never panics (well-formedness
    /// validation rejects such graphs up front, so this only guards the hot
    /// path). Semantics:
    ///
    /// - **Eq / Ne** — numeric on `(Number, Num)`, string on `(Text, Text)`.
    /// - **Gt / Ge / Lt / Le** — numeric only, on `(Number, Num)`.
    /// - **Contains** — `(Text, Text)`, substring of the value in the context.
    /// - **In** — membership in a [`RouteValue::List`]. On a `Text` context this
    ///   is string equality against each entry; on a `Number` context each list
    ///   entry is **parsed to `f64`** and compared numerically (a non-numeric
    ///   entry simply never matches). This parse-based numeric membership is the
    ///   documented choice.
    /// - **Between** — numeric only, `lo ≤ x ≤ hi` against a
    ///   [`RouteValue::Range`].
    pub fn eval(self, ctx: CtxValue, val: &RouteValue) -> bool {
        use RouteOp::*;
        match self {
            Eq => match (&ctx, val) {
                (CtxValue::Number(x), RouteValue::Num(y)) => x == y,
                (CtxValue::Text(s), RouteValue::Text(t)) => s == t,
                _ => false,
            },
            Ne => match (&ctx, val) {
                (CtxValue::Number(x), RouteValue::Num(y)) => x != y,
                (CtxValue::Text(s), RouteValue::Text(t)) => s != t,
                _ => false,
            },
            Gt => num_cmp(&ctx, val, |x, y| x > y),
            Ge => num_cmp(&ctx, val, |x, y| x >= y),
            Lt => num_cmp(&ctx, val, |x, y| x < y),
            Le => num_cmp(&ctx, val, |x, y| x <= y),
            Contains => match (&ctx, val) {
                (CtxValue::Text(s), RouteValue::Text(t)) => s.contains(t.as_str()),
                _ => false,
            },
            In => match (&ctx, val) {
                (CtxValue::Text(s), RouteValue::List(items)) => items.iter().any(|it| it == s),
                (CtxValue::Number(x), RouteValue::List(items)) => items
                    .iter()
                    .any(|it| it.parse::<f64>().map(|n| n == *x).unwrap_or(false)),
                _ => false,
            },
            Between => match (&ctx, val) {
                (CtxValue::Number(x), RouteValue::Range { lo, hi }) => *lo <= *x && *x <= *hi,
                _ => false,
            },
        }
    }
}

/// Helper for the four ordering operators: apply `cmp` only on a
/// `(Number, Num)` pair, else `false`.
fn num_cmp(ctx: &CtxValue, val: &RouteValue, cmp: impl Fn(f64, f64) -> bool) -> bool {
    match (ctx, val) {
        (CtxValue::Number(x), RouteValue::Num(y)) => cmp(*x, *y),
        _ => false,
    }
}

/// A literal on the right-hand side of a routing condition.
///
/// The variant a condition carries is constrained by its operator (validated):
/// ordering/equality on numerics use [`RouteValue::Num`]; string equality and
/// `Contains` use [`RouteValue::Text`]; [`RouteOp::In`] uses [`RouteValue::List`];
/// [`RouteOp::Between`] uses [`RouteValue::Range`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RouteValue {
    /// A numeric literal.
    Num(f64),
    /// A text/enum literal.
    Text(String),
    /// A set of string literals, for [`RouteOp::In`].
    List(Vec<String>),
    /// An inclusive numeric range `[lo, hi]`, for [`RouteOp::Between`].
    Range {
        /// Lower bound (inclusive).
        lo: f64,
        /// Upper bound (inclusive).
        hi: f64,
    },
}

/// The value a [`crate::RoutingContext`] yields for one field — the left-hand
/// side handed to [`RouteOp::eval`]. Numeric fields produce [`CtxValue::Number`];
/// string/enum fields produce [`CtxValue::Text`].
#[derive(Clone, Debug, PartialEq)]
pub enum CtxValue {
    /// A numeric field value.
    Number(f64),
    /// A string/enum field value.
    Text(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_kinds_are_correct() {
        for f in [
            RouteField::Side,
            RouteField::Product,
            RouteField::Ccy,
            RouteField::Counterparty,
            RouteField::User,
            RouteField::Desk,
        ] {
            assert_eq!(f.kind(), FieldKind::Enum, "{f:?} should be Enum");
        }
        for f in [
            RouteField::Notional,
            RouteField::Tenor,
            RouteField::Strike,
            RouteField::Price,
        ] {
            assert_eq!(f.kind(), FieldKind::Numeric, "{f:?} should be Numeric");
        }
        assert_eq!(RouteField::InstrumentId.kind(), FieldKind::String);
    }

    #[test]
    fn valid_for_matrix() {
        use RouteOp::*;
        // Numeric: ordering + between, no contains/in.
        for op in [Eq, Ne, Gt, Ge, Lt, Le, Between] {
            assert!(op.valid_for(FieldKind::Numeric), "{op:?} numeric");
        }
        for op in [Contains, In] {
            assert!(!op.valid_for(FieldKind::Numeric), "{op:?} !numeric");
        }
        // String: eq/ne/contains/in, no ordering/between.
        for op in [Eq, Ne, Contains, In] {
            assert!(op.valid_for(FieldKind::String), "{op:?} string");
        }
        for op in [Gt, Ge, Lt, Le, Between] {
            assert!(!op.valid_for(FieldKind::String), "{op:?} !string");
        }
        // Enum: eq/ne/in, no contains/ordering/between.
        for op in [Eq, Ne, In] {
            assert!(op.valid_for(FieldKind::Enum), "{op:?} enum");
        }
        for op in [Gt, Ge, Lt, Le, Between, Contains] {
            assert!(!op.valid_for(FieldKind::Enum), "{op:?} !enum");
        }
    }

    #[test]
    fn eval_numeric_ops() {
        let x = CtxValue::Number(10.0);
        assert!(RouteOp::Eq.eval(x.clone(), &RouteValue::Num(10.0)));
        assert!(!RouteOp::Eq.eval(x.clone(), &RouteValue::Num(11.0)));
        assert!(RouteOp::Ne.eval(x.clone(), &RouteValue::Num(11.0)));
        assert!(RouteOp::Gt.eval(x.clone(), &RouteValue::Num(9.0)));
        assert!(!RouteOp::Gt.eval(x.clone(), &RouteValue::Num(10.0)));
        assert!(RouteOp::Ge.eval(x.clone(), &RouteValue::Num(10.0)));
        assert!(RouteOp::Lt.eval(x.clone(), &RouteValue::Num(11.0)));
        assert!(!RouteOp::Lt.eval(x.clone(), &RouteValue::Num(10.0)));
        assert!(RouteOp::Le.eval(x.clone(), &RouteValue::Num(10.0)));
    }

    #[test]
    fn eval_between_endpoints_inclusive() {
        let range = RouteValue::Range { lo: 5.0, hi: 10.0 };
        assert!(RouteOp::Between.eval(CtxValue::Number(5.0), &range));
        assert!(RouteOp::Between.eval(CtxValue::Number(10.0), &range));
        assert!(RouteOp::Between.eval(CtxValue::Number(7.5), &range));
        assert!(!RouteOp::Between.eval(CtxValue::Number(4.999), &range));
        assert!(!RouteOp::Between.eval(CtxValue::Number(10.001), &range));
    }

    #[test]
    fn eval_string_ops() {
        let s = CtxValue::Text("EURUSD".to_string());
        assert!(RouteOp::Eq.eval(s.clone(), &RouteValue::Text("EURUSD".into())));
        assert!(!RouteOp::Eq.eval(s.clone(), &RouteValue::Text("GBPUSD".into())));
        assert!(RouteOp::Ne.eval(s.clone(), &RouteValue::Text("GBPUSD".into())));
        assert!(RouteOp::Contains.eval(s.clone(), &RouteValue::Text("EUR".into())));
        assert!(!RouteOp::Contains.eval(s.clone(), &RouteValue::Text("JPY".into())));
    }

    #[test]
    fn eval_in_text_membership() {
        let cp = CtxValue::Text("HF-2".to_string());
        let list = RouteValue::List(vec!["HF-1".into(), "HF-2".into()]);
        assert!(RouteOp::In.eval(cp, &list));
        assert!(!RouteOp::In.eval(CtxValue::Text("HF-9".into()), &list));
    }

    #[test]
    fn eval_in_numeric_parses_entries() {
        let n = CtxValue::Number(3.0);
        let list = RouteValue::List(vec!["1".into(), "3".into(), "nope".into()]);
        assert!(RouteOp::In.eval(n, &list));
        assert!(!RouteOp::In.eval(CtxValue::Number(2.0), &list));
        // Non-numeric entries never match a numeric context.
        assert!(!RouteOp::In.eval(CtxValue::Number(0.0), &RouteValue::List(vec!["x".into()])));
    }

    #[test]
    fn eval_type_mismatch_is_false_never_panics() {
        // Numeric op on a text context.
        assert!(!RouteOp::Gt.eval(CtxValue::Text("a".into()), &RouteValue::Num(1.0)));
        // Equality across types.
        assert!(!RouteOp::Eq.eval(CtxValue::Number(1.0), &RouteValue::Text("1".into())));
        assert!(!RouteOp::Eq.eval(CtxValue::Text("1".into()), &RouteValue::Num(1.0)));
        // Ne across types is also false (not "true because different") — total.
        assert!(!RouteOp::Ne.eval(CtxValue::Number(1.0), &RouteValue::Text("1".into())));
        // Contains on numeric.
        assert!(!RouteOp::Contains.eval(CtxValue::Number(1.0), &RouteValue::Text("1".into())));
        // Between with a non-range value.
        assert!(!RouteOp::Between.eval(CtxValue::Number(1.0), &RouteValue::Num(1.0)));
        // In with a non-list value.
        assert!(!RouteOp::In.eval(CtxValue::Text("a".into()), &RouteValue::Text("a".into())));
    }

    #[test]
    fn eval_empty_string_boundary() {
        let empty = CtxValue::Text(String::new());
        assert!(RouteOp::Eq.eval(empty.clone(), &RouteValue::Text(String::new())));
        // Every string contains the empty substring.
        assert!(RouteOp::Contains.eval(
            CtxValue::Text("abc".into()),
            &RouteValue::Text(String::new())
        ));
        assert!(!RouteOp::In.eval(empty, &RouteValue::List(vec!["a".into()])));
    }
}
