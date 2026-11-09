//! Namespace documents: the JSON form of one namespace, as
//! `docs/specs/namespace-documents.md` specifies.
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
//! its own error. Sungma reads each expression, checks the namespace whole,
//! and reports every problem it finds, up to [`MAX_ERRORS`]. [`print()`]
//! writes a namespace back as a document that parses as an equal namespace.

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

pub use sungma::name::{MAX_NAMESPACE_BYTES, MAX_QUOTE_BYTES, MAX_RELATION_BYTES};
use sungma::{
    name::{NameError, NamespaceName, RelationName, quote},
    namespace::{MAX_REWRITE_DEPTH, Namespace, NamespaceError, Problem},
    non_empty::NonEmpty,
    rewrite::Rewrite,
};

pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RELATIONS: usize = 500;
pub const MAX_EXPRESSION_BYTES: usize = 4096;
/// How many errors a refused document reports before [`ErrorKind::TooManyErrors`].
pub const MAX_ERRORS: usize = 50;

/// One namespace as a document declares it, its relations in document order.
#[derive(Debug)]
pub struct NamespaceDocument {
    pub namespace: NamespaceName,
    pub relations: Namespace<RelationName>,
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
    #[error("a document doesn't start with a byte-order mark")]
    ByteOrderMark,
    #[error("a document is at most {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
    #[error("a document is a JSON object keyed by its namespace's name")]
    NotAnObject,
    #[error("a document declares no namespace")]
    NoNamespace,
    #[error("a document declares one namespace, not {0}")]
    ManyNamespaces(usize),
    #[error("namespace '{0}' maps to an object of relations")]
    RelationsNotAnObject(String),
    #[error("a namespace declares at most {MAX_RELATIONS} relations, not {0}")]
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
    Namespace(NamespaceError),
    #[error("too many errors; the first {MAX_ERRORS} are reported")]
    TooManyErrors,
    #[error("namespace '{0}' is declared twice")]
    DuplicateNamespace(String),
}

/// Parses and checks a namespace document, as the [module](self) describes.
///
/// Linear in the document's length: one JSON pass, one pass over each
/// expression's lexemes, and [`Namespace::new`]'s linear check. A document
/// past [`MAX_RELATIONS`] is refused after the JSON pass alone.
pub fn parse(text: &str) -> Result<NamespaceDocument, Vec<DocumentError>> {
    let fail = |kind| Err(vec![error(None, None, kind)]);
    if text.len() > MAX_DOCUMENT_BYTES {
        return fail(ErrorKind::TooLarge);
    }
    let json = match serde_json::from_str::<Document>(text) {
        Ok(json) => json,
        Err(json) => return fail(ErrorKind::Json(json)),
    };
    let Json::Object(mut namespaces) = json else {
        return fail(ErrorKind::NotAnObject);
    };
    let (namespace, relations) = match namespaces.len() {
        0 => return fail(ErrorKind::NoNamespace),
        1 => namespaces.remove(0),
        count => return fail(ErrorKind::ManyNamespaces(count)),
    };
    let relations = match relations {
        Json::Object(entries) => Ok(entries),
        Json::Other => Err(ErrorKind::RelationsNotAnObject(quote(namespace.text()))),
    };
    read(namespace, relations)
}

/// Reads a namespace from what serde_json read: its name, and its
/// relations' names and values in document order, or the error that refuses
/// what the name maps to.
fn read(
    namespace: Text<'_>,
    relations: Result<Vec<(Text<'_>, Leaf<'_>)>, ErrorKind>,
) -> Result<NamespaceDocument, Vec<DocumentError>> {
    let mut errors = Errors::default();
    // A name that fails is still kept, unchecked, for the checks that
    // follow; the document is refused, so it never leaves.
    let namespace = match namespace {
        Text::Plain(name) => name.parse().unwrap_or_else(|error| {
            errors.push(None, None, ErrorKind::Name(error));
            NamespaceName::new_unchecked(name)
        }),
        Text::Escaped(name) => {
            errors.push(None, None, ErrorKind::Escape(quote(&name)));
            NamespaceName::new_unchecked(name)
        }
    };
    let entries = match relations {
        Ok(entries) => entries,
        Err(kind) => {
            errors.push(None, None, kind);
            return Err(errors.finish().unwrap_or_default());
        }
    };
    let declared = entries
        .iter()
        .map(|(name, _)| name.text())
        .collect::<HashSet<_>>()
        .len();
    if declared > MAX_RELATIONS {
        // Nothing past the limit is read, so a hostile document costs no
        // more than its library's pass over it.
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
        // so the namespace reports nothing more about it.
        let rewrite = match value {
            Leaf::Text(Text::Plain("")) => Err((None, ErrorKind::NotAnExpression)),
            Leaf::Text(Text::Plain(expression)) if expression.len() > MAX_EXPRESSION_BYTES => {
                Err((None, ErrorKind::ExpressionTooLong))
            }
            Leaf::Text(Text::Plain(expression)) => {
                Parser::new(&lex(expression, Mode::Expression)).parse()
            }
            Leaf::Text(Text::Escaped(expression)) => {
                Err((None, ErrorKind::Escape(quote(&expression))))
            }
            Leaf::Other => Err((None, ErrorKind::NotAnExpression)),
        };
        let rewrite = rewrite.unwrap_or_else(|(place, kind)| {
            let column = place.map(|place| place.column);
            errors.at(at, name.as_str(), column, kind);
            Rewrite::This
        });
        relations.push((name, rewrite));
    }
    let names: Vec<_> = relations.iter().map(|(name, _)| name.clone()).collect();
    let relations = Namespace::new(relations).map_err(|problems| {
        for Problem { at, error } in problems {
            let kind = ErrorKind::Namespace(quoted(error));
            errors.at(Some(at), names[at].as_str(), None, kind);
        }
    });
    match (errors.finish(), relations) {
        (None, Ok(relations)) => Ok(NamespaceDocument {
            namespace,
            relations,
        }),
        (errors, _) => Err(errors.unwrap_or_default()),
    }
}

/// Every problem a refused namespace document was found with.
#[derive(Debug, Error)]
#[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
pub struct Refusal(pub Vec<DocumentError>);

fn error(relation: Option<&str>, column: Option<usize>, kind: ErrorKind) -> DocumentError {
    DocumentError {
        relation: relation.map(quote),
        column,
        kind,
    }
}

/// Errors in document order: those of the namespace first, then each
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

/// A namespace problem with the names it carries quoted.
pub(crate) fn quoted(problem: NamespaceError) -> NamespaceError {
    match problem {
        NamespaceError::DuplicateRelation(relation) => {
            NamespaceError::DuplicateRelation(quote(&relation))
        }
        NamespaceError::DanglingReference { relation, target } => {
            NamespaceError::DanglingReference {
                relation: quote(&relation),
                target: quote(&target),
            }
        }
        NamespaceError::RewriteCycle { path, omitted } => NamespaceError::RewriteCycle {
            path: path.iter().map(|relation| quote(relation)).collect(),
            omitted,
        },
        other => other,
    }
}

/// A document's string, borrowed from the document when it holds no escape.
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

/// A document: namespaces, each an object of relations, each a [`Leaf`].
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
pub(crate) enum Token {
    /// The keyword `this`.
    This,
    Name,
    Open,
    Close,
    Comma,
    Union,
    Intersection,
    Exclusion,
    Colon,
    Equals,
    /// A character no token starts with.
    Unknown,
    End,
}

/// Where a lexeme starts: its line, and its column counting characters from
/// 1. An expression is one line, however many line breaks it holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub struct Place {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct Lexeme<'a> {
    pub(crate) token: Token,
    pub(crate) text: &'a str,
    pub(crate) place: Place,
    /// The byte offsets of `text` in what was lexed.
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Lexeme<'_> {
    fn found(&self) -> String {
        match self.token {
            Token::End => "the end".to_owned(),
            _ => format!("'{}'", quote(self.text)),
        }
    }
}

/// What is being lexed: one expression from a JSON string, or a
/// whole document in Sungma schema language, which has lines and comments.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Expression,
    Schema,
}

pub(crate) fn lex(text: &str, mode: Mode) -> Vec<Lexeme<'_>> {
    let mut lexemes = Vec::new();
    let mut chars = text.char_indices().peekable();
    let (mut line, mut column) = (1, 0);
    while let Some((start, c)) = chars.next() {
        column += 1;
        let place = Place { line, column };
        let token = match c {
            '\n' if mode == Mode::Schema => {
                (line, column) = (line + 1, 0);
                continue;
            }
            ' ' | '\t' | '\n' | '\r' => continue,
            '/' if mode == Mode::Schema && matches!(chars.peek(), Some((_, '/'))) => {
                while chars.next_if(|&(_, c)| c != '\n').is_some() {
                    column += 1;
                }
                continue;
            }
            '(' => Token::Open,
            ')' => Token::Close,
            ',' => Token::Comma,
            '|' => Token::Union,
            '&' => Token::Intersection,
            '!' => Token::Exclusion,
            ':' => Token::Colon,
            '=' => Token::Equals,
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut end = start + 1;
                while let Some((at, _)) =
                    chars.next_if(|&(_, c)| c.is_ascii_alphanumeric() || c == '_')
                {
                    end = at + 1;
                    column += 1;
                }
                let text = &text[start..end];
                let token = match text {
                    "this" => Token::This,
                    _ => Token::Name,
                };
                lexemes.push(Lexeme {
                    token,
                    text,
                    place,
                    start,
                    end,
                });
                continue;
            }
            _ => Token::Unknown,
        };
        let end = start + c.len_utf8();
        lexemes.push(Lexeme {
            token,
            text: &text[start..end],
            place,
            start,
            end,
        });
    }
    lexemes.push(Lexeme {
        token: Token::End,
        text: "",
        place: Place {
            line,
            column: column + 1,
        },
        start: text.len(),
        end: text.len(),
    });
    lexemes
}

/// What a term starts with.
const TERM: &str = "'this', a relation name or '('";

/// A syntax error's place and kind.
pub(crate) type Failure = (Option<Place>, ErrorKind);

type Node = Rewrite<RelationName>;

/// Recursive descent over the rewrite grammar, one function per level.
pub(crate) struct Parser<'l, 'a> {
    lexemes: &'l [Lexeme<'a>],
    at: usize,
    /// Where the expression ends: the parser reads the lexeme here as the
    /// end, whatever it is.
    end: usize,
    groups: usize,
}

impl<'l, 'a> Parser<'l, 'a> {
    /// A parser for one whole expression, `lexemes`, which end in
    /// [`Token::End`].
    pub(crate) fn new(lexemes: &'l [Lexeme<'a>]) -> Self {
        Self::within(lexemes, 0, lexemes.len() - 1)
    }

    /// A parser for the expression from lexeme `start` up to lexeme `end`.
    pub(crate) fn within(lexemes: &'l [Lexeme<'a>], start: usize, end: usize) -> Self {
        Self {
            lexemes,
            at: start,
            end,
            groups: 0,
        }
    }

    /// A whole expression: a rewrite, then the end.
    pub(crate) fn parse(mut self) -> Result<Rewrite<RelationName>, Failure> {
        let rewrite = self.union()?;
        let next = self.peek();
        if next.token != Token::End {
            return Err(syntax(next, "an operator or the end"));
        }
        Ok(rewrite)
    }

    fn peek(&self) -> Lexeme<'a> {
        let lexeme = self.lexemes[self.at];
        if self.at == self.end {
            Lexeme {
                token: Token::End,
                ..lexeme
            }
        } else {
            lexeme
        }
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
        node: fn(NonEmpty<Rewrite<RelationName>>) -> Rewrite<RelationName>,
    ) -> Result<Node, Failure> {
        let first = operand(self)?;
        if self.peek().token != operator {
            return Ok(first);
        }
        let mut rest = Vec::new();
        while self.peek().token == operator {
            self.next();
            rest.push(operand(self)?);
        }
        Ok(node(NonEmpty::with_rest(first, rest)))
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
                    return Err((Some(lexeme.place), ErrorKind::TooManyParentheses));
                }
                let node = self.union()?;
                self.close()?;
                self.groups -= 1;
                Ok(node)
            }
            _ => Err(syntax(lexeme, TERM)),
        }
    }

    /// Whether the `(` just read opens `(factset, computed)`.
    fn peek_is_fact_to(&self) -> bool {
        matches!(
            self.lexemes[self.at..self.end],
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
        const EXPECTED: &str = "a relation name";
        let lexeme = self.next();
        match lexeme.token {
            Token::Name => relation(lexeme),
            _ => Err(syntax(lexeme, EXPECTED)),
        }
    }

    fn close(&mut self) -> Result<(), Failure> {
        const EXPECTED: &str = "')'";
        let lexeme = self.next();
        match lexeme.token {
            Token::Close => Ok(()),
            _ => Err(syntax(lexeme, EXPECTED)),
        }
    }
}

pub(crate) fn syntax(lexeme: Lexeme<'_>, expected: &'static str) -> Failure {
    let found = lexeme.found();
    (Some(lexeme.place), ErrorKind::Syntax { found, expected })
}

/// A name token as a relation name, refused at its place if it isn't one.
pub(crate) fn relation(lexeme: Lexeme<'_>) -> Result<RelationName, Failure> {
    lexeme
        .text
        .parse()
        .map_err(|error| (Some(lexeme.place), ErrorKind::Name(error)))
}

/// Why [`print()`] refused a namespace: no document can
/// declare it.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PrintError {
    /// A name made with `new_unchecked` that breaks the grammar, such as
    /// `this`.
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("a union or intersection has one operand")]
    OneOperand,
    #[error("a schema document declares at least one namespace")]
    NoNamespace,
    #[error("namespace '{0}' isn't in the theory")]
    OutsideTheory(String),
    #[error("namespace '{0}' is given twice")]
    DuplicateNamespace(String),
    #[error("the printed document would be more than {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
    #[error("a printed rule would be more than {MAX_EXPRESSION_BYTES} bytes")]
    ExpressionTooLong,
}

/// Writes a namespace as a document that [`parse`] reads back as an equal
/// namespace: relations sorted by name, two-space indents, `\n` line ends.
pub fn print(
    namespace: &NamespaceName,
    relations: &Namespace<RelationName>,
) -> Result<String, PrintError> {
    let relations = printed(namespace, relations)?;
    let mut text = format!("{{\n  \"{namespace}\": {{");
    for (i, (name, expression)) in relations.iter().enumerate() {
        let separator = if i == 0 { "\n" } else { ",\n" };
        let _ = write!(text, "{separator}    \"{name}\": \"{expression}\"");
    }
    if relations.is_empty() {
        text.push_str("}\n}\n");
    } else {
        text.push_str("\n  }\n}\n");
    }
    Ok(text)
}

/// A namespace's relations sorted by name, each with its expression, once
/// every name is one a document can declare.
pub(crate) fn printed<'a>(
    namespace: &NamespaceName,
    relations: &'a Namespace<RelationName>,
) -> Result<Vec<(&'a str, String)>, PrintError> {
    namespace.as_str().parse::<NamespaceName>()?;
    let mut relations: Vec<_> = relations.relations().collect();
    relations.sort_by_key(|(name, _)| *name);
    relations
        .into_iter()
        .map(|(name, rewrite)| {
            let name = printable(name)?;
            let mut text = String::new();
            expression(&mut text, rewrite)?;
            Ok((name, text))
        })
        .collect()
}

fn expression(text: &mut String, rewrite: &Rewrite<RelationName>) -> Result<(), PrintError> {
    match rewrite {
        Rewrite::This => text.push_str("this"),
        Rewrite::Computed(relation) => text.push_str(printable(relation)?),
        Rewrite::FactTo { factset, computed } => {
            let _ = write!(text, "({}, {})", printable(factset)?, printable(computed)?);
        }
        Rewrite::Union(operands) | Rewrite::Intersection(operands) if operands.len() < 2 => {
            return Err(PrintError::OneOperand);
        }
        Rewrite::Union(operands) => operator(text, operands, " | ", |operand| {
            matches!(operand, Rewrite::Union(_))
        })?,
        Rewrite::Intersection(operands) => operator(text, operands, " & ", |operand| {
            matches!(operand, Rewrite::Union(_) | Rewrite::Intersection(_))
        })?,
        Rewrite::Exclusion(base, excluded) => {
            grouped(
                text,
                base,
                matches!(**base, Rewrite::Union(_) | Rewrite::Intersection(_)),
            )?;
            text.push_str(" ! ");
            grouped(
                text,
                excluded,
                matches!(
                    **excluded,
                    Rewrite::Union(_) | Rewrite::Intersection(_) | Rewrite::Exclusion(..)
                ),
            )?;
        }
    }
    Ok(())
}

/// `name`, if a document can declare a relation by it.
fn printable(name: &RelationName) -> Result<&str, NameError> {
    name.as_str().parse::<RelationName>()?;
    Ok(name.as_str())
}

fn operator(
    text: &mut String,
    operands: &[Rewrite<RelationName>],
    symbol: &str,
    needs_parentheses: fn(&Rewrite<RelationName>) -> bool,
) -> Result<(), PrintError> {
    for (i, operand) in operands.iter().enumerate() {
        if i > 0 {
            text.push_str(symbol);
        }
        grouped(text, operand, needs_parentheses(operand))?;
    }
    Ok(())
}

fn grouped(
    text: &mut String,
    rewrite: &Rewrite<RelationName>,
    parenthesize: bool,
) -> Result<(), PrintError> {
    if parenthesize {
        text.push('(');
        expression(text, rewrite)?;
        text.push(')');
    } else {
        expression(text, rewrite)?;
    }
    Ok(())
}
