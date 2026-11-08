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
//! are those of [namespace documents](crate::namespace). The expression limit
//! holds for a relation's rule as written, from its first token to its last.
//! A namespace name that already starts with the theory's name and a dot is
//! taken as written in full.

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

    fn token(&self, at: usize) -> Token {
        self.lexemes
            .get(at)
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
        self.keyword_at(self.at, keyword)
    }

    /// Whether `keyword:` starts at lexeme `at`.
    fn keyword_at(&self, at: usize, keyword: &str) -> bool {
        let lexeme = self.lexemes[at];
        lexeme.token == Token::Name && lexeme.text == keyword && self.token(at + 1) == Token::Colon
    }

    /// Whether a relation starts next: a name, then `=` or `:`.
    fn at_relation(&self) -> bool {
        self.relation_at(self.at)
    }

    /// Whether a relation starts at lexeme `at`.
    fn relation_at(&self, at: usize) -> bool {
        matches!(self.lexemes[at].token, Token::Name | Token::This)
            && matches!(self.token(at + 1), Token::Equals | Token::Colon)
    }

    /// How many relations the namespace whose relations start next
    /// declares, counted without reading them.
    fn declarations(&self) -> usize {
        (self.at..self.lexemes.len())
            .take_while(|&at| self.token(at) != Token::End && !self.keyword_at(at, "namespace"))
            .filter(|&at| self.relation_at(at))
            .count()
    }

    /// Whether what comes next ends a relation: another relation, a
    /// namespace, or the end.
    fn at_boundary(&self) -> bool {
        self.token(self.at) == Token::End || self.at_relation()
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
        // A theory name that fails prefixes nothing, so it's reported once.
        let theory = if self.at_keyword("theory") {
            self.next();
            self.next();
            self.namespace_name(None).ok()
        } else {
            None
        };
        let mut namespaces = Vec::new();
        let mut seen = HashSet::new();
        loop {
            if self.at_keyword("namespace") {
                let place = self.next().place;
                self.next();
                // A name that fails is reported once, and isn't a duplicate
                // of another that fails.
                let name = match self.namespace_name(theory.as_ref()) {
                    Ok(name) => {
                        if !seen.insert(name.clone()) {
                            let kind = ErrorKind::DuplicateNamespace(quote(name.as_str()));
                            self.push(Some(&name), None, (Some(place), kind));
                        }
                        name
                    }
                    Err(name) => name,
                };
                if let Some(relations) = self.relations(&name, place) {
                    namespaces.push(NamespaceDocument {
                        namespace: name,
                        relations,
                    });
                }
                // Every error from here on comes later in the document than
                // those already found, so once they pass the cap, reading on
                // would find none that are reported.
                if self.errors.len() > MAX_ERRORS {
                    break;
                }
            } else if self.peek().token == Token::End {
                break;
            } else {
                let failure = syntax(self.next(), NAMESPACE_OR_END);
                self.push(None, None, failure);
                self.skip_until(|reader| reader.at_keyword("namespace"));
            }
        }
        if namespaces.is_empty() && self.errors.is_empty() {
            let place = Some(self.peek().place);
            self.push(None, None, (place, ErrorKind::NoNamespace));
        }
        namespaces
    }

    /// A namespace name, its segments written without spaces between them,
    /// with the theory's name before it unless it's written in full. A name
    /// that fails is reported, and kept, unchecked, as the error, for the
    /// checks that follow; the document is refused, so it never leaves.
    fn namespace_name(
        &mut self,
        theory: Option<&NamespaceName>,
    ) -> Result<NamespaceName, NamespaceName> {
        const EXPECTED: &str = "a namespace name";
        let first = self.peek();
        // A missing name leaves the next header or relation alone.
        if self.at_relation() {
            self.push(None, None, syntax(first, EXPECTED));
            return Err(NamespaceName::new_unchecked(""));
        }
        let mut end = first.start;
        // Everything written without a space is the name, so a malformed
        // name is refused whole, as one name.
        for lexeme in &self.lexemes[self.at..] {
            if lexeme.token == Token::End || lexeme.start != end {
                break;
            }
            end = lexeme.end;
            self.at += 1;
        }
        if end == first.start {
            let failure = syntax(self.next(), EXPECTED);
            self.push(None, None, failure);
            return Err(NamespaceName::new_unchecked(""));
        }
        let written = &self.text[first.start..end];
        let name = match theory {
            Some(theory) if !written.starts_with(&format!("{theory}.")) => {
                format!("{theory}.{written}")
            }
            _ => written.to_owned(),
        };
        name.parse().map_err(|error| {
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
        // Every declaration counts, a relation declared twice included, and
        // nothing past the limit is read, so a hostile namespace costs no
        // more than one pass over it.
        let declared = self.declarations();
        if declared > MAX_RELATIONS {
            let kind = ErrorKind::TooManyRelations(declared);
            self.push(Some(namespace), None, (Some(header), kind));
            self.skip_until(|reader| reader.at_keyword("namespace"));
            return None;
        }
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
        // The rule runs to where the next relation or namespace starts, or
        // to the end. As in the JSON form, its text is held to the limit
        // before it's parsed, so a hostile rule builds no deep tree.
        let start = self.at;
        self.recover();
        let end = self.at;
        let (first, last) = (self.lexemes[start], self.lexemes[end - 1]);
        let parsed = if end > start && last.end - first.start > MAX_EXPRESSION_BYTES {
            Err((Some(first.place), ErrorKind::ExpressionTooLong))
        } else {
            Parser::within(self.lexemes, start, end).parse()
        };
        parsed.unwrap_or_else(|failure| {
            self.push(Some(namespace), Some(name.as_str()), failure);
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
/// A rule that prints longer than [`MAX_EXPRESSION_BYTES`], as a dense rule
/// spaced out can, and a document longer than [`MAX_DOCUMENT_BYTES`], are
/// refused, since [`parse`] would refuse them.
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
        // A name that still starts with the theory once it's taken off is
        // written in full, as the parser reads it.
        let written = match &prefix {
            Some(prefix) => match name.as_str().strip_prefix(prefix.as_str()) {
                Some(rest) if rest.starts_with(prefix.as_str()) => name.as_str(),
                Some(rest) => rest,
                None => return Err(PrintError::OutsideTheory(quote(name.as_str()))),
            },
            None => name.as_str(),
        };
        let relations = printed(name, relations)?;
        if !text.is_empty() {
            text.push('\n');
        }
        let _ = writeln!(text, "namespace: {written}");
        for (relation, expression) in relations {
            if expression.len() > MAX_EXPRESSION_BYTES {
                return Err(PrintError::ExpressionTooLong);
            }
            let _ = writeln!(text, "  {relation} = {expression}");
        }
        // Checked as it grows, so a caller's oversized input is refused
        // without being printed whole.
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(PrintError::TooLarge);
        }
    }
    Ok(text)
}
