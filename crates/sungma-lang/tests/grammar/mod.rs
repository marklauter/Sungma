//! The rewrite grammar's cases, the oracle every form's tests share: each
//! form parses every valid case to its tree and prints it canonically, and
//! refuses every invalid case where it goes wrong.

use sungma::{
    name::{MAX_RELATION_BYTES, RelationName},
    non_empty::NonEmpty,
    rewrite::Rewrite::{self, Computed, Exclusion, FactTo, Intersection, This, Union},
};

/// Relations every grammar case may name, each declared as `this`.
pub const DECLARED: [&str; 12] = [
    "a", "b", "c", "d", "p", "_a", "a1", "_", "thisone", "this_", "_this", "THIS1",
];

fn name(name: &str) -> Rewrite<RelationName> {
    Computed(RelationName::new_unchecked(name))
}

fn fact_to(factset: &str, computed: &str) -> Rewrite<RelationName> {
    FactTo {
        factset: RelationName::new_unchecked(factset),
        computed: RelationName::new_unchecked(computed),
    }
}

fn union(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Union(NonEmpty::new(operands).expect("at least one operand"))
}

fn intersection(operands: Vec<Rewrite<RelationName>>) -> Rewrite<RelationName> {
    Intersection(NonEmpty::new(operands).expect("at least one operand"))
}

fn exclusion(
    base: Rewrite<RelationName>,
    excluded: Rewrite<RelationName>,
) -> Rewrite<RelationName> {
    Exclusion(Box::new(base), Box::new(excluded))
}

/// Expressions the grammar accepts, each with the tree it parses to and the
/// text the printer writes for that tree.
pub fn valid() -> Vec<(&'static str, Rewrite<RelationName>, &'static str)> {
    let (a, b, c, d) = (name("a"), name("b"), name("c"), name("d"));
    vec![
        ("this", This, "this"),
        ("a", a.clone(), "a"),
        ("(p, a)", fact_to("p", "a"), "(p, a)"),
        ("(p,a)", fact_to("p", "a"), "(p, a)"),
        ("( p , a )", fact_to("p", "a"), "(p, a)"),
        ("((p, a))", fact_to("p", "a"), "(p, a)"),
        (
            "(p, undeclared)",
            fact_to("p", "undeclared"),
            "(p, undeclared)",
        ),
        ("(this)", This, "this"),
        ("((((a))))", a.clone(), "a"),
        ("this ! this", exclusion(This, This), "this ! this"),
        ("this & this", intersection(vec![This, This]), "this & this"),
        ("this|this", union(vec![This, This]), "this | this"),
        ("a|b", union(vec![a.clone(), b.clone()]), "a | b"),
        ("(a)|(b)", union(vec![a.clone(), b.clone()]), "a | b"),
        (
            "a | b & c | d",
            union(vec![
                a.clone(),
                intersection(vec![b.clone(), c.clone()]),
                d.clone(),
            ]),
            "a | b & c | d",
        ),
        (
            "a & b ! c & d",
            intersection(vec![a.clone(), exclusion(b.clone(), c.clone()), d.clone()]),
            "a & b ! c & d",
        ),
        (
            "a ! b & c",
            intersection(vec![exclusion(a.clone(), b.clone()), c.clone()]),
            "a ! b & c",
        ),
        (
            "a ! (b ! c)",
            exclusion(a.clone(), exclusion(b.clone(), c.clone())),
            "a ! (b ! c)",
        ),
        (
            "(a ! b) ! c",
            exclusion(exclusion(a.clone(), b.clone()), c.clone()),
            "a ! b ! c",
        ),
        (
            "(a | b) & c",
            intersection(vec![union(vec![a.clone(), b.clone()]), c.clone()]),
            "(a | b) & c",
        ),
        (
            "a & (b | c)",
            intersection(vec![a.clone(), union(vec![b.clone(), c.clone()])]),
            "a & (b | c)",
        ),
        (
            "(a & b) | c",
            union(vec![intersection(vec![a.clone(), b.clone()]), c.clone()]),
            "a & b | c",
        ),
        (
            "a | (b | c)",
            union(vec![a.clone(), union(vec![b.clone(), c.clone()])]),
            "a | (b | c)",
        ),
        (
            "(a & b) & c",
            intersection(vec![intersection(vec![a.clone(), b.clone()]), c.clone()]),
            "(a & b) & c",
        ),
        (
            "a ! (b & c)",
            exclusion(a.clone(), intersection(vec![b.clone(), c.clone()])),
            "a ! (b & c)",
        ),
        (
            "(a | b) ! c",
            exclusion(union(vec![a.clone(), b.clone()]), c.clone()),
            "(a | b) ! c",
        ),
        (
            "(a ! b) & (c ! d)",
            intersection(vec![
                exclusion(a.clone(), b.clone()),
                exclusion(c.clone(), d.clone()),
            ]),
            "a ! b & c ! d",
        ),
        (
            "(p, a) ! (p, b)",
            exclusion(fact_to("p", "a"), fact_to("p", "b")),
            "(p, a) ! (p, b)",
        ),
        (
            "a ! (p, b)",
            exclusion(a.clone(), fact_to("p", "b")),
            "a ! (p, b)",
        ),
        (
            "(p, a) & (p, b) | this",
            union(vec![
                intersection(vec![fact_to("p", "a"), fact_to("p", "b")]),
                This,
            ]),
            "(p, a) & (p, b) | this",
        ),
        (
            "a ! b ! c & d ! a | b",
            union(vec![
                intersection(vec![
                    exclusion(exclusion(a.clone(), b.clone()), c.clone()),
                    exclusion(d.clone(), a.clone()),
                ]),
                b.clone(),
            ]),
            "a ! b ! c & d ! a | b",
        ),
        (
            "_a | a1 | _ | thisone | this_ | _this | THIS1",
            union(
                ["_a", "a1", "_", "thisone", "this_", "_this", "THIS1"]
                    .into_iter()
                    .map(name)
                    .collect(),
            ),
            "_a | a1 | _ | thisone | this_ | _this | THIS1",
        ),
    ]
}

pub const TERM: &str = "'this', a relation name or '('";
pub const END: &str = "an operator or the end";
const CLOSE: &str = "')'";
const RELATION: &str = "a relation name";

/// Expressions the grammar refuses, each with the column, counting
/// characters from 1, where it goes wrong, what is found there, and what was
/// expected. A form whose rewrites aren't followed by the end expects more
/// than [`END`] where an expression could have ended.
pub fn invalid() -> Vec<(&'static str, usize, &'static str, &'static str)> {
    vec![
        ("   ", 4, "the end", TERM),
        ("| a", 1, "'|'", TERM),
        ("& a", 1, "'&'", TERM),
        ("! a", 1, "'!'", TERM),
        ("a !", 4, "the end", TERM),
        ("a &", 4, "the end", TERM),
        ("a || b", 4, "'|'", TERM),
        ("a !! b", 4, "'!'", TERM),
        ("a & | b", 5, "'|'", TERM),
        ("a ! & b", 5, "'&'", TERM),
        ("a)", 2, "')'", END),
        (")a", 1, "')'", TERM),
        ("()", 2, "')'", TERM),
        ("((a)", 5, "the end", CLOSE),
        ("(a))", 4, "')'", END),
        ("(a)(b)", 4, "'('", END),
        ("a (b)", 3, "'('", END),
        ("this(a)", 5, "'('", END),
        ("a b", 3, "'b'", END),
        ("this this", 6, "'this'", END),
        ("a,b", 2, "','", END),
        ("(,a)", 2, "','", TERM),
        ("(a,)", 4, "')'", RELATION),
        ("(a, b, c)", 6, "','", CLOSE),
        ("(a b, c)", 4, "'b'", CLOSE),
        ("(a | b, c)", 7, "','", CLOSE),
        ("(a, (p, b))", 5, "'('", RELATION),
        ("(this, a)", 2, "'this'", RELATION),
        ("(a, this)", 5, "'this'", RELATION),
        ("(p, a", 6, "the end", CLOSE),
        ("a.b", 2, "'.'", END),
        ("1a", 1, "'1'", TERM),
        ("a#b", 2, "'#'", END),
        ("a:b", 2, "':'", END),
        ("a-b", 2, "'-'", END),
        ("a | b ; c", 7, "';'", END),
        ("a ~ b", 3, "'~'", END),
        ("a | b/", 6, "'/'", END),
    ]
}

/// Expressions that name a reserved or overlong relation, each with the
/// column of that name.
pub fn reserved() -> Vec<(String, usize)> {
    let long = "n".repeat(MAX_RELATION_BYTES + 1);
    let fact_to_long = format!("a & (p, {long})");
    let cases = [
        ("THIS", 1),
        ("a | This", 5),
        ("(p, tHIS)", 5),
        ("(THIS, a)", 2),
        (long.as_str(), 1),
        (fact_to_long.as_str(), 9),
    ];
    cases
        .into_iter()
        .map(|(expression, column)| (expression.to_owned(), column))
        .collect()
}
