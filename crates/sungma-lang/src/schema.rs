//! Sungma schema language: a form of one or more namespaces written for
//! people, as `docs/specs/theory-documents.md` specifies.
//!
//! ```text
//! theory: io
//!
//! namespace: file
//!   owner = this
//!   parent = this
//!   viewer = this | owner | (parent, viewer)
//! ```
//!
//! A relation is its name, `=` and its rewrite, which may wrap over as many
//! lines as it needs: line breaks only separate tokens. A theory name, when
//! there is one, prefixes each namespace's name. Accepted subjects, written
//! `owner: identity`, come with accepted subjects in the namespace model.
//!
//! The rewrite grammar, the namespace checks, the limits and the error kinds
//! are those of [namespace documents](crate::namespace).

use std::collections::HashSet;
use std::fmt::Write;

use thiserror::Error;

use sungma::{
    name::{NamespaceName, RelationName, quote},
    namespace::{Namespace, Problem},
    rewrite::Rewrite,
};

use crate::namespace::{
    ErrorKind, Failure, Lexeme, MAX_DOCUMENT_BYTES, MAX_ERRORS, MAX_EXPRESSION_BYTES,
    MAX_RELATIONS, Mode, NamespaceDocument, Parser, Place, PrintError, Token, lex, printed, quoted,
    relation, syntax,
};

/// A problem in a schema document, with the namespace and relation it was
/// found in, and its place in the document.
#[derive(Debug, Error)]
#[error("{}{}{kind}", Self::at(*.place), Self::within(.namespace, .relation))]
pub struct SchemaError {
    pub namespace: Option<String>,
    pub relation: Option<String>,
    pub place: Option<Place>,
    pub kind: ErrorKind,
}

impl SchemaError {
    fn at(place: Option<Place>) -> String {
        place.map_or_else(String::new, |Place { line, column }| {
            format!("line {line}, column {column}: ")
        })
    }

    fn within(namespace: &Option<String>, relation: &Option<String>) -> String {
        match (namespace, relation) {
            (Some(namespace), Some(relation)) => {
                format!("namespace '{namespace}', relation '{relation}': ")
            }
            (Some(namespace), None) => format!("namespace '{namespace}': "),
            _ => String::new(),
        }
    }
}

const NAMESPACE_OR_END: &str = "'namespace:' or the end";
const RELATION: &str = "a relation, 'namespace:' or the end";
const AFTER_REWRITE: &str = "an operator, a relation, 'namespace:' or the end";

/// Parses and checks a document in Sungma schema language, as the
/// [module](self) describes, into its namespaces under their full names.
///
/// Linear in the document's length: one pass to lex it, one to parse it, and
/// [`Namespace::new`]'s linear check of each namespace.
pub fn parse(text: &str) -> Result<Vec<NamespaceDocument>, Vec<SchemaError>> {
    let fail = |kind| {
        Err(vec![SchemaError {
            namespace: None,
            relation: None,
            place: None,
            kind,
        }])
    };
    if text.len() > MAX_DOCUMENT_BYTES {
        return fail(ErrorKind::TooLarge);
    }
    if text.starts_with('\u{feff}') {
        return fail(ErrorKind::ByteOrderMark);
    }
    let lexemes = lex(text, Mode::Schema);
    let mut reader = Reader {
        text,
        lexemes: &lexemes,
        at: 0,
        errors: Vec::new(),
    };
    let namespaces = reader.document();
    match reader.finish() {
        None => Ok(namespaces),
        Some(errors) => Err(errors),
    }
}

struct Reader<'l, 'a> {
    text: &'a str,
    lexemes: &'l [Lexeme<'a>],
    at: usize,
    errors: Vec<SchemaError>,
}

impl<'a> Reader<'_, 'a> {
    fn peek(&self) -> Lexeme<'a> {
        self.lexemes[self.at]
    }

    fn token(&self, ahead: usize) -> Token {
        self.lexemes
            .get(self.at + ahead)
            .map_or(Token::End, |lexeme| lexeme.token)
    }

    fn next(&mut self) -> Lexeme<'a> {
        let lexeme = self.peek();
        if lexeme.token != Token::End {
            self.at += 1;
        }
        lexeme
    }

    /// Whether `keyword:` comes next.
    fn at_keyword(&self, keyword: &str) -> bool {
        let lexeme = self.peek();
        lexeme.token == Token::Name && lexeme.text == keyword && self.token(1) == Token::Colon
    }

    /// Whether a relation starts next: a name, then `=` or `:`.
    fn at_relation(&self) -> bool {
        matches!(self.token(0), Token::Name | Token::This)
            && matches!(self.token(1), Token::Equals | Token::Colon)
    }

    /// Whether what comes next ends a relation: another relation, a
    /// namespace, or the end.
    fn at_boundary(&self) -> bool {
        self.token(0) == Token::End || self.at_relation()
    }

    fn push(
        &mut self,
        namespace: Option<&NamespaceName>,
        relation: Option<&str>,
        (place, kind): Failure,
    ) {
        self.errors.push(SchemaError {
            namespace: namespace.map(|name| quote(name.as_str())),
            relation: relation.map(quote),
            place,
            kind,
        });
    }

    /// Skips past a problem to where the next relation or namespace starts.
    fn recover(&mut self) {
        self.skip_until(Self::at_boundary);
    }

    /// Skips lexemes until `stop` holds or the end. The end bounds the loop
    /// whatever `stop` says.
    fn skip_until(&mut self, stop: fn(&Self) -> bool) {
        let end = self.lexemes.len() - 1;
        for at in self.at..end {
            self.at = at;
            if stop(self) {
                return;
            }
        }
        self.at = end;
    }

    fn document(&mut self) -> Vec<NamespaceDocument> {
        let theory = if self.at_keyword("theory") {
            self.next();
            self.next();
            Some(self.namespace_name(None))
        } else {
            None
        };
        let mut namespaces = Vec::new();
        let mut seen = HashSet::new();
        loop {
            if self.at_keyword("namespace") {
                let place = self.next().place;
                self.next();
                let name = self.namespace_name(theory.as_ref());
                if !seen.insert(name.clone()) {
                    let kind = ErrorKind::DuplicateNamespace(quote(name.as_str()));
                    self.push(Some(&name), None, (Some(place), kind));
                }
                if let Some(relations) = self.relations(&name, place) {
                    namespaces.push(NamespaceDocument {
                        namespace: name,
                        relations,
                    });
                }
            } else if self.peek().token == Token::End {
                break;
            } else {
                let failure = syntax(self.next(), NAMESPACE_OR_END);
                self.push(None, None, failure);
                self.skip_until(|reader| reader.at_keyword("namespace"));
            }
        }
        if namespaces.is_empty() && seen.is_empty() && self.errors.is_empty() {
            let place = Some(self.peek().place);
            self.push(None, None, (place, ErrorKind::NoNamespace));
        }
        namespaces
    }

    /// A namespace name, its segments written without spaces between them,
    /// with the theory's name before it. A name that fails is kept,
    /// unchecked, for the checks that follow; the document is refused, so it
    /// never leaves.
    fn namespace_name(&mut self, theory: Option<&NamespaceName>) -> NamespaceName {
        let first = self.peek();
        let mut end = first.start;
        for lexeme in &self.lexemes[self.at..] {
            let segment = matches!(lexeme.token, Token::Name | Token::This | Token::Dot);
            if !(segment && lexeme.start == end) {
                break;
            }
            end = lexeme.end;
            self.at += 1;
        }
        if end == first.start {
            let failure = syntax(self.next(), "a namespace name");
            self.push(None, None, failure);
            return NamespaceName::new_unchecked("");
        }
        let written = &self.text[first.start..end];
        let name = match theory {
            Some(theory) => format!("{theory}.{written}"),
            None => written.to_owned(),
        };
        name.parse().unwrap_or_else(|error| {
            self.push(None, None, (Some(first.place), ErrorKind::Name(error)));
            NamespaceName::new_unchecked(name)
        })
    }

    /// A namespace's relations, up to the next namespace or the end, checked
    /// whole, or `None` when the checks refuse it. Any error refuses the whole
    /// document, so a namespace returned beside one never leaves.
    fn relations(
        &mut self,
        namespace: &NamespaceName,
        header: Place,
    ) -> Option<Namespace<RelationName>> {
        let mut relations = Vec::new();
        let mut places = Vec::new();
        while !(self.peek().token == Token::End || self.at_keyword("namespace")) {
            if !self.at_relation() {
                let failure = syntax(self.next(), RELATION);
                self.push(Some(namespace), None, failure);
                self.recover();
                continue;
            }
            let lexeme = self.next();
            let name = relation(lexeme).unwrap_or_else(|failure| {
                self.push(Some(namespace), Some(lexeme.text), failure);
                RelationName::new_unchecked(lexeme.text)
            });
            let rewrite = self.rewrite(namespace, &name);
            places.push(lexeme.place);
            relations.push((name, rewrite));
        }
        let declared = relations
            .iter()
            .map(|(name, _)| name)
            .collect::<HashSet<_>>()
            .len();
        if declared > MAX_RELATIONS {
            let kind = ErrorKind::TooManyRelations(declared);
            self.push(Some(namespace), None, (Some(header), kind));
            return None;
        }
        let names: Vec<_> = relations.iter().map(|(name, _)| name.clone()).collect();
        match Namespace::new(relations) {
            Ok(relations) => Some(relations),
            Err(problems) => {
                for Problem { at, error } in problems {
                    let kind = ErrorKind::Namespace(quoted(error));
                    let failure = (Some(places[at]), kind);
                    self.push(Some(namespace), Some(names[at].as_str()), failure);
                }
                None
            }
        }
    }

    /// The `=` and rewrite of relation `name`, whose name was just read. A
    /// relation whose rewrite fails is still declared, as `this`, so the
    /// namespace reports nothing more about it.
    fn rewrite(&mut self, namespace: &NamespaceName, name: &RelationName) -> Rewrite<RelationName> {
        let equals = self.next();
        if equals.token != Token::Equals {
            let failure = syntax(equals, "'='");
            self.push(Some(namespace), Some(name.as_str()), failure);
            self.recover();
            return Rewrite::This;
        }
        let first = self.peek();
        let mut parser = Parser::at(self.lexemes, self.at);
        let parsed = parser.rewrite();
        self.at = parser.at;
        let parsed = parsed.and_then(|rewrite| {
            let last = self.lexemes[self.at.saturating_sub(1)];
            if last.end.saturating_sub(first.start) > MAX_EXPRESSION_BYTES {
                Err((Some(first.place), ErrorKind::ExpressionTooLong))
            } else if self.at_boundary() {
                Ok(rewrite)
            } else {
                Err(syntax(self.peek(), AFTER_REWRITE))
            }
        });
        parsed.unwrap_or_else(|failure| {
            self.push(Some(namespace), Some(name.as_str()), failure);
            self.recover();
            Rewrite::This
        })
    }

    /// Every error in document order, capped at [`MAX_ERRORS`], or `None`.
    fn finish(&mut self) -> Option<Vec<SchemaError>> {
        if self.errors.is_empty() {
            return None;
        }
        let mut errors = std::mem::take(&mut self.errors);
        errors.sort_by_key(|error| error.place);
        if errors.len() > MAX_ERRORS {
            errors.truncate(MAX_ERRORS);
            errors.push(SchemaError {
                namespace: None,
                relation: None,
                place: None,
                kind: ErrorKind::TooManyErrors,
            });
        }
        Some(errors)
    }
}

/// Writes namespaces as a document in Sungma schema language that [`parse`]
/// reads back as equal namespaces. With a theory name, every namespace's
/// name starts with it and a dot, and is written without them. Namespaces
/// and their relations are sorted by name, relations are indented by two
/// spaces, a blank line comes before each namespace, and lines end in `\n`.
pub fn print(
    theory: Option<&NamespaceName>,
    namespaces: &[(&NamespaceName, &Namespace<RelationName>)],
) -> Result<String, PrintError> {
    if namespaces.is_empty() {
        return Err(PrintError::NoNamespace);
    }
    let mut namespaces = namespaces.to_vec();
    namespaces.sort_by_key(|(name, _)| name.as_str());
    let mut text = String::new();
    if let Some(theory) = theory {
        theory.as_str().parse::<NamespaceName>()?;
        let _ = writeln!(text, "theory: {theory}");
    }
    let prefix = theory.map(|theory| format!("{theory}."));
    let mut previous: Option<&str> = None;
    for (name, relations) in namespaces {
        if previous == Some(name.as_str()) {
            return Err(PrintError::DuplicateNamespace(quote(name.as_str())));
        }
        previous = Some(name.as_str());
        let written = match &prefix {
            Some(prefix) => name
                .as_str()
                .strip_prefix(prefix.as_str())
                .ok_or_else(|| PrintError::OutsideTheory(quote(name.as_str())))?,
            None => name.as_str(),
        };
        let relations = printed(name, relations)?;
        if !text.is_empty() {
            text.push('\n');
        }
        let _ = writeln!(text, "namespace: {written}");
        for (relation, expression) in relations {
            let _ = writeln!(text, "  {relation} = {expression}");
        }
    }
    Ok(text)
}
