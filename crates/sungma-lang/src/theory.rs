//! Theory documents: the JSON form of one theory, as
//! `docs/specs/theory-documents.md` specifies.
//!
//! ```json
//! {
//!   "file": {
//!     "owner": "this",
//!     "parent": "this",
//!     "editor": "this | owner",
//!     "viewer": "(this | editor | (parent, viewer)) ! banned",
//!     "banned": "this"
//!   }
//! }
//! ```
//!
//! serde_json reads the JSON, and refuses a document that isn't JSON with
//! its own error. Sungma reads each expression, checks the theory whole, and
//! reports every problem it finds, up to [`MAX_ERRORS`]. [`print()`] writes a
//! theory back as a document that parses as an equal theory.

use std::{
    collections::HashSet,
    fmt::{self, Write},
    marker::PhantomData,
};

use serde::{
    Deserialize, Deserializer,
    de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use thiserror::Error;

pub use sungma::name::{MAX_NAME_BYTES, MAX_QUOTE_BYTES};
use sungma::{
    id::{RelationId, TheoryId},
    memory::{MemoryDictionary, MemoryTheoryStore},
    name::{NameError, RelationName, TheoryName, quote},
    rewrite::Rewrite,
    store::Pool,
    theory::{MAX_REWRITE_DEPTH, Problem, Theory, TheoryError},
};

pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RELATIONS: usize = 500;
pub const MAX_EXPRESSION_BYTES: usize = 4096;
/// How many errors a refused document reports before [`ErrorKind::TooManyErrors`].
pub const MAX_ERRORS: usize = 50;

/// One theory as a document declares it, its relations in document order.
#[derive(Debug)]
pub struct TheoryDocument {
    pub theory: TheoryName,
    pub relations: Theory<RelationName>,
}

/// A problem in a document, with the relation it was found at and, inside
/// an expression, the column where it starts, counting characters from 1.
#[derive(Debug, Error)]
#[error("{}{kind}", Self::place(.relation, *.column))]
pub struct DocumentError {
    pub relation: Option<String>,
    pub column: Option<usize>,
    pub kind: ErrorKind,
}

impl DocumentError {
    fn place(relation: &Option<String>, column: Option<usize>) -> String {
        match (relation, column) {
            (Some(relation), Some(column)) => format!("relation '{relation}', column {column}: "),
            (Some(relation), None) => format!("relation '{relation}': "),
            (None, _) => String::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ErrorKind {
    #[error("{0}")]
    Json(serde_json::Error),
    #[error("a document is at most {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
    #[error("a document is a JSON object keyed by its theory's name")]
    NotAnObject,
    #[error("a document declares no theory")]
    NoTheory,
    #[error("a document declares one theory, not {0}")]
    ManyTheories(usize),
    #[error("theory '{0}' maps to an object of relations")]
    RelationsNotAnObject(String),
    #[error("a theory declares at most {MAX_RELATIONS} relations, not {0}")]
    TooManyRelations(usize),
    #[error("'{0}' contains a JSON escape")]
    Escape(String),
    #[error(transparent)]
    Name(NameError),
    #[error("a relation maps to a non-empty expression string")]
    NotAnExpression,
    #[error("an expression is at most {MAX_EXPRESSION_BYTES} bytes")]
    ExpressionTooLong,
    #[error("expected {expected}, found {found}")]
    Syntax {
        found: String,
        expected: &'static str,
    },
    #[error("parentheses nest more than {MAX_REWRITE_DEPTH} deep")]
    TooManyParentheses,
    #[error(transparent)]
    Theory(TheoryError),
    #[error("too many errors; the first {MAX_ERRORS} are reported")]
    TooManyErrors,
}

/// Parses and checks a theory document, as the [module](self) describes.
pub fn parse(text: &str) -> Result<TheoryDocument, Vec<DocumentError>> {
    let fail = |kind| Err(vec![error(None, None, kind)]);
    if text.len() > MAX_DOCUMENT_BYTES {
        return fail(ErrorKind::TooLarge);
    }
    let json = match serde_json::from_str::<Document>(text) {
        Ok(json) => json,
        Err(json) => return fail(ErrorKind::Json(json)),
    };
    let Json::Object(mut theories) = json else {
        return fail(ErrorKind::NotAnObject);
    };
    let (theory, relations) = match theories.len() {
        0 => return fail(ErrorKind::NoTheory),
        1 => theories.remove(0),
        count => return fail(ErrorKind::ManyTheories(count)),
    };
    let mut errors = Errors::default();
    // A name that fails is still kept, unchecked, for the checks that
    // follow; the document is refused, so it never leaves.
    let theory = match theory {
        Text::Plain(name) => name.parse().unwrap_or_else(|error| {
            errors.push(None, None, ErrorKind::Name(error));
            TheoryName::new_unchecked(name)
        }),
        Text::Escaped(name) => {
            errors.push(None, None, ErrorKind::Escape(quote(&name)));
            TheoryName::new_unchecked(name)
        }
    };
    let Json::Object(entries) = relations else {
        errors.push(
            None,
            None,
            ErrorKind::RelationsNotAnObject(quote(theory.as_str())),
        );
        return Err(errors.finish().unwrap_or_default());
    };
    let declared = entries
        .iter()
        .map(|(name, _)| name.text())
        .collect::<HashSet<_>>()
        .len();
    if declared > MAX_RELATIONS {
        // Nothing past the limit is read, so a hostile document costs no
        // more than the JSON library's pass over it.
        errors.push(None, None, ErrorKind::TooManyRelations(declared));
        return Err(errors.finish().unwrap_or_default());
    }
    let mut relations = Vec::new();
    for (name, value) in entries {
        let at = Some(relations.len());
        let name = match name {
            Text::Plain(name) => name.parse().unwrap_or_else(|error| {
                errors.at(at, name, None, ErrorKind::Name(error));
                RelationName::new_unchecked(name)
            }),
            Text::Escaped(name) => {
                errors.at(at, &name, None, ErrorKind::Escape(quote(&name)));
                RelationName::new_unchecked(name)
            }
        };
        // A relation whose expression fails is still declared, as `this`,
        // so the theory reports nothing more about it.
        let rewrite = match value {
            Leaf::Text(Text::Plain("")) => Err((None, ErrorKind::NotAnExpression)),
            Leaf::Text(Text::Plain(expression)) if expression.len() > MAX_EXPRESSION_BYTES => {
                Err((None, ErrorKind::ExpressionTooLong))
            }
            Leaf::Text(Text::Plain(expression)) => Parser::new(expression).parse(),
            Leaf::Text(Text::Escaped(expression)) => {
                Err((None, ErrorKind::Escape(quote(&expression))))
            }
            Leaf::Other => Err((None, ErrorKind::NotAnExpression)),
        };
        let rewrite = rewrite.unwrap_or_else(|(column, kind)| {
            errors.at(at, name.as_str(), column, kind);
            Rewrite::This
        });
        relations.push((name, rewrite));
    }
    let names: Vec<_> = relations.iter().map(|(name, _)| name.clone()).collect();
    let relations = Theory::new(relations).map_err(|problems| {
        for Problem { at, error } in problems {
            let kind = ErrorKind::Theory(quoted(error));
            errors.at(Some(at), names[at].as_str(), None, kind);
        }
    });
    match (errors.finish(), relations) {
        (None, Ok(relations)) => Ok(TheoryDocument { theory, relations }),
        (errors, _) => Err(errors.unwrap_or_default()),
    }
}

/// Every problem a refused theory document was found with.
#[derive(Debug, Error)]
#[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
pub struct Refusal(pub Vec<DocumentError>);

/// Parses each document in turn and declares its theory, interning its
/// names in `dictionary`.
pub fn load_theories(
    documents: &[&str],
    dictionary: &MemoryDictionary,
    theories: &mut MemoryTheoryStore,
) -> Result<(), Refusal> {
    for document in documents {
        let document = parse(document).map_err(Refusal)?;
        let id = TheoryId(dictionary.intern(Pool::Theories, document.theory.as_str()));
        let relations = document
            .relations
            .map(|name| RelationId(dictionary.intern(Pool::Relations, name.as_str())));
        theories.declare(id, relations);
    }
    Ok(())
}

fn error(relation: Option<&str>, column: Option<usize>, kind: ErrorKind) -> DocumentError {
    DocumentError {
        relation: relation.map(quote),
        column,
        kind,
    }
}

/// Errors in document order: those of the theory first, then each
/// relation's in its order, then by column.
#[derive(Default)]
struct Errors(Vec<(Option<usize>, DocumentError)>);

impl Errors {
    fn push(&mut self, relation: Option<&str>, column: Option<usize>, kind: ErrorKind) {
        self.0.push((None, error(relation, column, kind)));
    }

    fn at(&mut self, at: Option<usize>, relation: &str, column: Option<usize>, kind: ErrorKind) {
        self.0.push((at, error(Some(relation), column, kind)));
    }

    fn finish(mut self) -> Option<Vec<DocumentError>> {
        if self.0.is_empty() {
            return None;
        }
        self.0.sort_by_key(|(at, error)| (*at, error.column));
        let mut errors: Vec<_> = self.0.into_iter().map(|(_, error)| error).collect();
        if errors.len() > MAX_ERRORS {
            errors.truncate(MAX_ERRORS);
            errors.push(error(None, None, ErrorKind::TooManyErrors));
        }
        Some(errors)
    }
}

/// A theory problem with the names it carries quoted.
fn quoted(problem: TheoryError) -> TheoryError {
    match problem {
        TheoryError::DuplicateRelation(relation) => {
            TheoryError::DuplicateRelation(quote(&relation))
        }
        TheoryError::DanglingReference { relation, target } => TheoryError::DanglingReference {
            relation: quote(&relation),
            target: quote(&target),
        },
        TheoryError::RewriteCycle { path, omitted } => TheoryError::RewriteCycle {
            path: path.iter().map(|relation| quote(relation)).collect(),
            omitted,
        },
        other => other,
    }
}

/// A JSON string, borrowed from the document when it holds no escape.
enum Text<'a> {
    Plain(&'a str),
    Escaped(String),
}

impl Text<'_> {
    fn text(&self) -> &str {
        match self {
            Text::Plain(text) => text,
            Text::Escaped(text) => text,
        }
    }
}

/// As much of a JSON value as a document's shape needs: an object's entries
/// in document order, duplicates kept, each value read as `V`, or anything
/// else.
enum Json<'a, V> {
    Object(Vec<(Text<'a>, V)>),
    Other,
}

/// A relation's value: its expression, or anything else, read no further.
enum Leaf<'a> {
    Text(Text<'a>),
    Other,
}

/// A document: theories, each an object of relations, each a [`Leaf`].
type Document<'a> = Json<'a, Json<'a, Leaf<'a>>>;

struct TextVisitor;

impl<'de> Visitor<'de> for TextVisitor {
    type Value = Text<'de>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string")
    }

    fn visit_borrowed_str<E: de::Error>(self, text: &'de str) -> Result<Self::Value, E> {
        Ok(Text::Plain(text))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
        Ok(Text::Escaped(text.to_owned()))
    }
}

impl<'de> Deserialize<'de> for Text<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(TextVisitor)
    }
}

/// Reads any JSON value: strings and objects through `text` and `object`,
/// arrays skipped unread, and everything else as `other`.
macro_rules! any_value {
    ($value:ty, $other:expr) => {
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<$value, E> {
            Ok($other)
        }

        fn visit_i64<E: de::Error>(self, _: i64) -> Result<$value, E> {
            Ok($other)
        }

        fn visit_u64<E: de::Error>(self, _: u64) -> Result<$value, E> {
            Ok($other)
        }

        fn visit_f64<E: de::Error>(self, _: f64) -> Result<$value, E> {
            Ok($other)
        }

        fn visit_unit<E: de::Error>(self) -> Result<$value, E> {
            Ok($other)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<$value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok($other)
        }
    };
}

struct JsonVisitor<V>(PhantomData<V>);

impl<'de, V: Deserialize<'de>> Visitor<'de> for JsonVisitor<V> {
    type Value = Json<'de, V>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(Json::Other)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(Json::Object(entries))
    }

    any_value!(Json<'de, V>, Json::Other);
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Json<'de, V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor(PhantomData))
    }
}

struct LeafVisitor;

impl<'de> Visitor<'de> for LeafVisitor {
    type Value = Leaf<'de>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_borrowed_str<E: de::Error>(self, text: &'de str) -> Result<Self::Value, E> {
        Ok(Leaf::Text(Text::Plain(text)))
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
        Ok(Leaf::Text(Text::Escaped(text.to_owned())))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Leaf::Other)
    }

    any_value!(Leaf<'de>, Leaf::Other);
}

impl<'de> Deserialize<'de> for Leaf<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(LeafVisitor)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    /// The keyword `this`.
    This,
    Name,
    Open,
    Close,
    Comma,
    Union,
    Intersection,
    Exclusion,
    /// A character no token starts with.
    Unknown,
    End,
}

#[derive(Clone, Copy)]
struct Lexeme<'a> {
    token: Token,
    text: &'a str,
    column: usize,
}

impl Lexeme<'_> {
    fn found(&self) -> String {
        match self.token {
            Token::End => "the end".to_owned(),
            _ => format!("'{}'", quote(self.text)),
        }
    }
}

fn lex(expression: &str) -> Vec<Lexeme<'_>> {
    let mut lexemes = Vec::new();
    let mut chars = expression.char_indices().enumerate().peekable();
    while let Some((column, (start, c))) = chars.next() {
        let column = column + 1;
        let token = match c {
            ' ' | '\t' | '\n' | '\r' => continue,
            '(' => Token::Open,
            ')' => Token::Close,
            ',' => Token::Comma,
            '|' => Token::Union,
            '&' => Token::Intersection,
            '!' => Token::Exclusion,
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut end = start + 1;
                while let Some(&(_, (at, c))) = chars.peek() {
                    if !(c.is_ascii_alphanumeric() || c == '_') {
                        break;
                    }
                    end = at + 1;
                    chars.next();
                }
                let text = &expression[start..end];
                let token = match text {
                    "this" => Token::This,
                    _ => Token::Name,
                };
                lexemes.push(Lexeme {
                    token,
                    text,
                    column,
                });
                continue;
            }
            _ => Token::Unknown,
        };
        let text = &expression[start..start + c.len_utf8()];
        lexemes.push(Lexeme {
            token,
            text,
            column,
        });
    }
    lexemes.push(Lexeme {
        token: Token::End,
        text: "",
        column: expression.chars().count() + 1,
    });
    lexemes
}

/// A syntax error's column and kind.
type Failure = (Option<usize>, ErrorKind);

type Node = Rewrite<RelationName>;

/// Recursive descent over the rewrite grammar, one function per level.
struct Parser<'a> {
    lexemes: Vec<Lexeme<'a>>,
    at: usize,
    groups: usize,
}

impl<'a> Parser<'a> {
    fn new(expression: &'a str) -> Self {
        Self {
            lexemes: lex(expression),
            at: 0,
            groups: 0,
        }
    }

    fn parse(mut self) -> Result<Rewrite<RelationName>, Failure> {
        let rewrite = self.union()?;
        let next = self.peek();
        if next.token != Token::End {
            return Err(syntax(next, "an operator or the end"));
        }
        Ok(rewrite)
    }

    fn peek(&self) -> Lexeme<'a> {
        self.lexemes[self.at]
    }

    fn next(&mut self) -> Lexeme<'a> {
        let lexeme = self.peek();
        if lexeme.token != Token::End {
            self.at += 1;
        }
        lexeme
    }

    fn union(&mut self) -> Result<Node, Failure> {
        self.run(Token::Union, Self::intersection, Rewrite::Union)
    }

    fn intersection(&mut self) -> Result<Node, Failure> {
        self.run(Token::Intersection, Self::exclusion, Rewrite::Intersection)
    }

    /// A run of one operator, as one node with an operand for each term.
    fn run(
        &mut self,
        operator: Token,
        operand: fn(&mut Self) -> Result<Node, Failure>,
        node: fn(Vec<Rewrite<RelationName>>) -> Rewrite<RelationName>,
    ) -> Result<Node, Failure> {
        let first = operand(self)?;
        if self.peek().token != operator {
            return Ok(first);
        }
        let mut operands = vec![first];
        while self.peek().token == operator {
            self.next();
            operands.push(operand(self)?);
        }
        Ok(node(operands))
    }

    fn exclusion(&mut self) -> Result<Node, Failure> {
        let mut base = self.term()?;
        while self.peek().token == Token::Exclusion {
            self.next();
            let excluded = self.term()?;
            base = Rewrite::Exclusion(Box::new(base), Box::new(excluded));
        }
        Ok(base)
    }

    fn term(&mut self) -> Result<Node, Failure> {
        let lexeme = self.next();
        match lexeme.token {
            Token::This => Ok(Rewrite::This),
            Token::Name => Ok(Rewrite::Computed(relation(lexeme)?)),
            Token::Open if self.peek_is_fact_to() => self.fact_to(),
            Token::Open => {
                self.groups += 1;
                if self.groups > MAX_REWRITE_DEPTH {
                    return Err((Some(lexeme.column), ErrorKind::TooManyParentheses));
                }
                let node = self.union()?;
                self.close()?;
                self.groups -= 1;
                Ok(node)
            }
            _ => Err(syntax(lexeme, "'this', a relation name or '('")),
        }
    }

    /// Whether the `(` just read opens `(factset, computed)`.
    fn peek_is_fact_to(&self) -> bool {
        matches!(
            self.lexemes[self.at..],
            [
                Lexeme {
                    token: Token::This | Token::Name,
                    ..
                },
                Lexeme {
                    token: Token::Comma,
                    ..
                },
                ..
            ]
        )
    }

    fn fact_to(&mut self) -> Result<Node, Failure> {
        let factset = self.relation()?;
        self.next();
        let computed = self.relation()?;
        self.close()?;
        Ok(Rewrite::FactTo { factset, computed })
    }

    fn relation(&mut self) -> Result<RelationName, Failure> {
        let lexeme = self.next();
        match lexeme.token {
            Token::Name => relation(lexeme),
            _ => Err(syntax(lexeme, "a relation name")),
        }
    }

    fn close(&mut self) -> Result<(), Failure> {
        let lexeme = self.next();
        match lexeme.token {
            Token::Close => Ok(()),
            _ => Err(syntax(lexeme, "')'")),
        }
    }
}

fn syntax(lexeme: Lexeme<'_>, expected: &'static str) -> Failure {
    let found = lexeme.found();
    (Some(lexeme.column), ErrorKind::Syntax { found, expected })
}

/// A name token as a relation name, refused at its column if it isn't one.
fn relation(lexeme: Lexeme<'_>) -> Result<RelationName, Failure> {
    lexeme
        .text
        .parse()
        .map_err(|error| (Some(lexeme.column), ErrorKind::Name(error)))
}

/// Writes a theory as a document that [`parse`] reads back as an equal
/// theory: relations sorted by name, two-space indents, `\n` line ends.
///
/// # Panics
///
/// On a caller defect, a theory no document can declare: a name made with
/// `new_unchecked` that breaks the grammar, including `this`, or a union or
/// intersection with one operand.
pub fn print(theory: &TheoryName, relations: &Theory<RelationName>) -> String {
    if let Err(error) = theory.as_str().parse::<TheoryName>() {
        panic!("{error}");
    }
    let mut relations: Vec<_> = relations.relations().collect();
    relations.sort_by_key(|(name, _)| *name);
    let mut text = format!("{{\n  \"{theory}\": {{");
    for (i, (name, rewrite)) in relations.iter().enumerate() {
        let separator = if i == 0 { "\n" } else { ",\n" };
        let _ = write!(text, "{separator}    \"{}\": \"", printable(name));
        expression(&mut text, rewrite);
        text.push('"');
    }
    if relations.is_empty() {
        text.push_str("}\n}\n");
    } else {
        text.push_str("\n  }\n}\n");
    }
    text
}

fn expression(text: &mut String, rewrite: &Rewrite<RelationName>) {
    match rewrite {
        Rewrite::This => text.push_str("this"),
        Rewrite::Computed(relation) => text.push_str(printable(relation)),
        Rewrite::FactTo { factset, computed } => {
            let _ = write!(text, "({}, {})", printable(factset), printable(computed));
        }
        Rewrite::Union(operands) | Rewrite::Intersection(operands) if operands.len() < 2 => {
            panic!("an operator with {} operands", operands.len())
        }
        Rewrite::Union(operands) => operator(text, operands, " | ", |operand| {
            matches!(operand, Rewrite::Union(_))
        }),
        Rewrite::Intersection(operands) => operator(text, operands, " & ", |operand| {
            matches!(operand, Rewrite::Union(_) | Rewrite::Intersection(_))
        }),
        Rewrite::Exclusion(base, excluded) => {
            grouped(
                text,
                base,
                matches!(**base, Rewrite::Union(_) | Rewrite::Intersection(_)),
            );
            text.push_str(" ! ");
            grouped(
                text,
                excluded,
                matches!(
                    **excluded,
                    Rewrite::Union(_) | Rewrite::Intersection(_) | Rewrite::Exclusion(..)
                ),
            );
        }
    }
}

/// `name`, if a document can declare a relation by it.
fn printable(name: &RelationName) -> &str {
    if let Err(error) = name.as_str().parse::<RelationName>() {
        panic!("{error}");
    }
    name.as_str()
}

fn operator(
    text: &mut String,
    operands: &[Rewrite<RelationName>],
    symbol: &str,
    needs_parentheses: fn(&Rewrite<RelationName>) -> bool,
) {
    for (i, operand) in operands.iter().enumerate() {
        if i > 0 {
            text.push_str(symbol);
        }
        grouped(text, operand, needs_parentheses(operand));
    }
}

fn grouped(text: &mut String, rewrite: &Rewrite<RelationName>, parenthesize: bool) {
    if parenthesize {
        text.push('(');
        expression(text, rewrite);
        text.push(')');
    } else {
        expression(text, rewrite);
    }
}
