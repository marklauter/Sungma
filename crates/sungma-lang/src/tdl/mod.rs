//! TDL, Theory Definition Language: documents of one or more namespaces,
//! as `docs/specs/theory-documents.md` specifies.
//!
//! ```text
//! theory: drive
//!
//! namespace: file
//!   owner<@identity>
//!   parent<folder>
//!   editor<@identity, ..group#member>: this | owner
//!   viewer: editor | (parent, viewer)
//! ```
//!
//! A relation reads like a generic type: its name, its constraints in angle
//! brackets, then a colon and its rewrite, which may wrap over as many
//! lines as it needs: line breaks only separate tokens. A relation with
//! constraints and no rewrite reads `this`, and one with a rewrite and no
//! constraints is computed. A rewrite runs to the next relation, a name
//! followed by `<` or `:`, to the next `namespace:` or `theory:`, or to the
//! end. `namespace` and `theory` are reserved relation names, so a header
//! is never read as a relation.
//!
//! A `theory:` line opens a section: its name and a dot go before each
//! namespace name in a header or a constraint, until the next `theory:`
//! line. In a constraint, `..` before a namespace name marks a full name
//! outside the theory. Namespaces before the first `theory:` line take no
//! prefix, and there `..` changes nothing.
//!
//! A reference from one namespace to another in the document, through a
//! constraint or a fact-to-subjectset, is checked against the document. A
//! reference to a namespace outside it is left to the namespace write.

mod print;
mod rewrite;

use std::collections::{HashMap, HashSet};

use thiserror::Error;

pub use print::print;
pub use rewrite::{ErrorKind, Place};
pub use sungma::{
    name::{MAX_NAMESPACE_BYTES, MAX_QUOTE_BYTES, MAX_RELATION_BYTES},
    namespace::DEFAULT_REWRITE_DEPTH,
};
use sungma::{
    name::{NameError, NamespaceName, RelationName, quote},
    namespace::{Constraint, Namespace, Problem},
    rewrite::Rewrite,
};

use rewrite::{Failure, Lexeme, Parser, Token, lex, quoted, reads_this, syntax};

/// How many errors a refused document reports, by default, before
/// [`ErrorKind::TooManyErrors`].
pub const DEFAULT_MAX_ERRORS: usize = 50;

/// The identity constraint. A name can't start with `@`, so it can never
/// be a namespace.
pub const IDENTITY: &str = "@identity";

/// What marks a constraint's namespace name as full, outside the theory.
pub const OUTSIDE: &str = "..";

/// A namespace whose relations and namespaces are named.
pub type Named = Namespace<RelationName, NamespaceName>;

/// A constraint as a document writes it, with names.
pub type NamedConstraint = Constraint<NamespaceName, RelationName>;

/// What the parser may be set to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    /// How deeply a rewrite nests, in levels and in grouping parentheses.
    /// The parser recurses once for each group, so a depth past the
    /// default needs a stack to match.
    pub depth: usize,
    /// How many errors a refused document reports before
    /// [`ErrorKind::TooManyErrors`].
    pub errors: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            depth: DEFAULT_REWRITE_DEPTH,
            errors: DEFAULT_MAX_ERRORS,
        }
    }
}

/// One namespace as a document declares it, under its full name, its
/// relations in document order.
#[derive(Debug)]
pub struct NamespaceDocument {
    pub namespace: NamespaceName,
    pub relations: Named,
}

/// A problem in a document, with the full name of the namespace and the
/// relation it was found in, and its place in the document.
#[derive(Debug, Error)]
#[error("{}{}{kind}", Self::at(*.place), Self::within(.namespace, .relation))]
pub struct TdlError {
    pub namespace: Option<String>,
    pub relation: Option<String>,
    pub place: Option<Place>,
    pub kind: ErrorKind,
}

impl TdlError {
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

/// Every problem a refused document was found with.
#[derive(Debug, Error)]
#[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
pub struct Refusal(pub Vec<TdlError>);

const HEADER_OR_END: &str = "'namespace:', 'theory:' or the end";
const RELATION: &str = "a relation, 'namespace:', 'theory:' or the end";
const CONSTRAINT: &str = "a constraint";
const AFTER_CONSTRAINT: &str = "',' or '>'";
const AFTER_CONSTRAINTS: &str = "':', a relation, 'namespace:', 'theory:' or the end";

/// Parses and checks a document, as the [module](self) describes, into its
/// namespaces under their full names, in document order, with the default
/// [`Settings`].
pub fn parse(text: &str) -> Result<Vec<NamespaceDocument>, Vec<TdlError>> {
    parse_with(text, Settings::default())
}

/// Like [`parse`], with `settings`.
///
/// Linear in the document's length: one pass to lex it, one to parse it, and
/// [`Namespace::with_depth`]'s linear check of each namespace.
pub fn parse_with(text: &str, settings: Settings) -> Result<Vec<NamespaceDocument>, Vec<TdlError>> {
    if text.starts_with('\u{feff}') {
        return Err(vec![unplaced(ErrorKind::ByteOrderMark)]);
    }
    let lexemes = lex(text);
    let mut reader = Reader {
        text,
        lexemes: &lexemes,
        at: 0,
        depth: settings.depth,
        cap: settings.errors,
        errors: Vec::new(),
    };
    let namespaces = reader.document();
    match reader.finish() {
        None => Ok(namespaces),
        Some(errors) => Err(errors),
    }
}

/// Parses a document that declares one namespace, such as a namespace
/// write's or a stored version's.
pub fn parse_one(text: &str) -> Result<NamespaceDocument, Vec<TdlError>> {
    let mut namespaces = parse(text)?;
    match namespaces.len() {
        1 => Ok(namespaces.remove(0)),
        count => Err(vec![unplaced(ErrorKind::ManyNamespaces(count))]),
    }
}

/// Whether a lexeme of `token` joins those written against it into one
/// name.
fn joins(token: Token) -> bool {
    matches!(token, Token::Name | Token::This | Token::Unknown)
}

fn unplaced(kind: ErrorKind) -> TdlError {
    TdlError {
        namespace: None,
        relation: None,
        place: None,
        kind,
    }
}

/// A namespace name of the grammar.
pub(crate) fn namespace_name(text: &str) -> Result<NamespaceName, NameError> {
    text.parse()
}

/// The problems with each reference, through a constraint or a
/// fact-to-subjectset, from one of `namespaces` to one of them, itself
/// included: one list for each namespace, in the order given. A namespace
/// not among them is left to the namespace write. A name given twice is
/// checked against the first.
pub(crate) fn reference_problems(namespaces: &[(&NamespaceName, &Named)]) -> Vec<Vec<Problem>> {
    let mut declared = HashMap::new();
    for &(name, relations) in namespaces {
        declared.entry(name).or_insert(relations);
    }
    namespaces
        .iter()
        .map(|&(own, relations)| {
            relations.references(own, |namespace, relation| {
                declared
                    .get(namespace)
                    .map(|relations| relations.rewrite(relation).is_some())
            })
        })
        .collect()
}

/// A constraint as a document writes it outside any theory: `@identity`,
/// a namespace name, or a namespace name and a relation name joined by
/// `#`, the namespace name optionally after `..`, which changes nothing
/// there. A malformed one is refused whole, quoting all of it.
pub fn constraint(text: &str) -> Result<NamedConstraint, NameError> {
    Theory::None.constraint(text)
}

/// The theory a document's section is in.
enum Theory {
    /// Before the first `theory:` line, where names are as written.
    None,
    Named(NamespaceName),
    /// A theory whose name failed, reported once. A name under it is checked
    /// as written and kept, unchecked, with the failed name before it; the
    /// document is refused, so it never leaves.
    Failed(String),
}

impl Theory {
    /// The full name of a namespace written as `written`, or the error that
    /// refuses it with the name kept, unchecked, for the checks that follow.
    fn qualify(&self, written: &str) -> Result<NamespaceName, (NameError, NamespaceName)> {
        let unchecked = |name: String| NamespaceName::new_unchecked(name);
        match self {
            Theory::None => {
                namespace_name(written).map_err(|error| (error, unchecked(written.to_owned())))
            }
            Theory::Named(theory) => {
                let full = format!("{theory}.{written}");
                namespace_name(&full).map_err(|error| (error, unchecked(full)))
            }
            Theory::Failed(theory) => {
                let full = unchecked(format!("{theory}.{written}"));
                match namespace_name(written) {
                    Ok(_) => Ok(full),
                    Err(error) => Err((error, full)),
                }
            }
        }
    }

    /// A constraint written in this theory, refused whole when malformed.
    fn constraint(&self, text: &str) -> Result<NamedConstraint, NameError> {
        if text == IDENTITY {
            return Ok(Constraint::Identity);
        }
        let whole = |error| match error {
            NameError::Invalid(_) => NameError::Invalid(quote(text)),
            other => other,
        };
        let namespace = |written: &str| {
            match written.strip_prefix(OUTSIDE) {
                Some(full) => namespace_name(full),
                None => self.qualify(written).map_err(|(error, _)| error),
            }
            .map_err(whole)
        };
        Ok(match text.split_once('#') {
            None => Constraint::Resource(namespace(text)?),
            Some((written, relation)) => {
                Constraint::Subjectset(namespace(written)?, relation.parse().map_err(whole)?)
            }
        })
    }
}

struct Reader<'l, 'a> {
    text: &'a str,
    lexemes: &'l [Lexeme<'a>],
    at: usize,
    depth: usize,
    /// How many errors are reported.
    cap: usize,
    errors: Vec<TdlError>,
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
        let lexeme = self.peek();
        lexeme.token == Token::Name
            && lexeme.text == keyword
            && self.token(self.at + 1) == Token::Colon
    }

    /// Whether a header, `namespace:` or `theory:`, comes next.
    fn at_header(&self) -> bool {
        self.at_keyword("namespace") || self.at_keyword("theory")
    }

    /// Whether a relation starts next: a name, read whole, then `<` or `:`.
    /// A header reads as one too, which only a relation named as a keyword
    /// could be. None starts inside a name, as at `rel` in `my-rel<`.
    fn at_relation(&self) -> bool {
        let first = self.peek();
        let inside = self
            .at
            .checked_sub(1)
            .map(|at| self.lexemes[at])
            .is_some_and(|before| joins(before.token) && before.end == first.start);
        matches!(first.token, Token::Name | Token::This)
            && !inside
            && matches!(self.token(self.run_end()), Token::Less | Token::Colon)
    }

    /// Whether what comes next ends a relation: another relation, a
    /// header, or the end.
    fn at_boundary(&self) -> bool {
        self.peek().token == Token::End || self.at_relation()
    }

    fn push(
        &mut self,
        namespace: Option<&NamespaceName>,
        relation: Option<&str>,
        (place, kind): Failure,
    ) {
        self.errors.push(TdlError {
            namespace: namespace.map(|name| quote(name.as_str())),
            relation: relation.map(quote),
            place,
            kind,
        });
        // Errors can arrive out of document order, as a namespace's checks
        // follow its syntax errors, so past the cap the earliest are kept,
        // one more than the cap to show it was passed. A stable sort keeps
        // errors at the same place in the order found, so those kept are
        // the first that [`finish`](Self::finish) would sort them all into.
        if self.errors.len() > 2 * self.cap {
            self.errors.sort_by_key(|error| error.place);
            self.errors.truncate(self.cap + 1);
        }
    }

    /// Skips past a problem to where the next relation or header starts.
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

    /// The lexemes written with no space between them from here, read as
    /// one name, so a malformed name is refused whole: the first lexeme and
    /// the text. `None`, with nothing read, when no name starts here, or
    /// when a relation does, so a missing name leaves the next relation or
    /// header alone.
    fn joined(&mut self) -> Option<(Lexeme<'a>, &'a str)> {
        if !joins(self.peek().token) || self.at_relation() {
            return None;
        }
        Some(self.run())
    }

    /// The lexemes written with no space between them from here, which
    /// start with one that [`joins`], read as one name: the first lexeme
    /// and the text.
    fn run(&mut self) -> (Lexeme<'a>, &'a str) {
        let first = self.peek();
        let end = self.run_end();
        self.at = end;
        (first, &self.text[first.start..self.lexemes[end - 1].end])
    }

    /// Where the lexemes written with no space between them from here end:
    /// the first lexeme past them. Each of them is read only from the first,
    /// as [`at_relation`](Self::at_relation) reads, so finding where every
    /// relation starts is linear.
    fn run_end(&self) -> usize {
        let mut end = self.peek().start;
        let mut next = self.at;
        for lexeme in &self.lexemes[self.at..] {
            if !joins(lexeme.token) || lexeme.start != end {
                break;
            }
            end = lexeme.end;
            next += 1;
        }
        next
    }

    fn document(&mut self) -> Vec<NamespaceDocument> {
        let mut theory = Theory::None;
        let mut namespaces: Vec<(NamespaceDocument, Vec<Place>)> = Vec::new();
        let mut seen = HashSet::new();
        // Each pass reads at least one lexeme, so the lexemes bound the passes
        // however the conditions inside read.
        for _ in 0..self.lexemes.len() {
            // Every error from here on comes later in the document than
            // those already found, so once they pass the cap, reading on
            // would find none that are reported.
            if self.errors.len() > self.cap {
                break;
            }
            if self.at_keyword("theory") {
                theory = self.theory();
            } else if self.at_keyword("namespace") {
                let place = self.next().place;
                self.next();
                // A name that fails is reported once, and isn't a duplicate
                // of another that fails.
                let name = match self.namespace_name(&theory) {
                    Ok(name) => {
                        if !seen.insert(name.clone()) {
                            let kind = ErrorKind::DuplicateNamespace(quote(name.as_str()));
                            self.push(Some(&name), None, (Some(place), kind));
                        }
                        name
                    }
                    Err(name) => name,
                };
                if let Some((relations, places)) = self.relations(&name, &theory) {
                    let document = NamespaceDocument {
                        namespace: name,
                        relations,
                    };
                    namespaces.push((document, places));
                }
            } else if self.peek().token == Token::End {
                break;
            } else {
                let failure = syntax(self.next(), HEADER_OR_END);
                self.push(None, None, failure);
                self.skip_until(Self::at_header);
            }
        }
        if namespaces.is_empty() && self.errors.is_empty() {
            let place = Some(self.peek().place);
            self.push(None, None, (place, ErrorKind::NoNamespace));
        }
        self.references(&namespaces);
        namespaces
            .into_iter()
            .map(|(document, _)| document)
            .collect()
    }

    /// A `theory:` line, which opens a section: the theory its namespaces
    /// are in.
    fn theory(&mut self) -> Theory {
        let header = self.next().place;
        self.next();
        let Some((first, written)) = self.joined() else {
            let failure = syntax(self.peek(), "a theory name");
            self.push(None, None, failure);
            return Theory::Failed(String::new());
        };
        let place = Some(first.place);
        if written.contains('.') {
            let kind = ErrorKind::DottedTheory(quote(written));
            self.push(None, None, (place, kind));
            return Theory::Failed(written.to_owned());
        }
        let name = match written.parse::<NamespaceName>() {
            Ok(name) => name,
            Err(error) => {
                self.push(None, None, (place, ErrorKind::Name(error)));
                return Theory::Failed(written.to_owned());
            }
        };
        // Anything else after the name is reported where it is, as the
        // document goes on.
        if self.peek().token == Token::End || self.at_keyword("theory") {
            let kind = ErrorKind::EmptyTheory(quote(name.as_str()));
            self.push(None, None, (Some(header), kind));
        }
        Theory::Named(name)
    }

    /// A namespace's full name, or, when it fails, reported, the name kept
    /// unchecked for the checks that follow; the document is refused, so it
    /// never leaves.
    fn namespace_name(&mut self, theory: &Theory) -> Result<NamespaceName, NamespaceName> {
        let Some((first, written)) = self.joined() else {
            let failure = syntax(self.peek(), "a namespace name");
            self.push(None, None, failure);
            return Err(NamespaceName::new_unchecked(""));
        };
        theory.qualify(written).map_err(|(error, name)| {
            let failure = (Some(first.place), ErrorKind::Name(error));
            self.push(Some(&name), None, failure);
            name
        })
    }

    /// Checks each reference from one namespace to another against the
    /// document. A namespace outside it is left to the namespace write.
    fn references(&mut self, namespaces: &[(NamespaceDocument, Vec<Place>)]) {
        let given: Vec<_> = namespaces
            .iter()
            .map(|(document, _)| (&document.namespace, &document.relations))
            .collect();
        for ((document, places), problems) in namespaces.iter().zip(reference_problems(&given)) {
            let (own, relations) = (&document.namespace, &document.relations);
            let names: Vec<_> = relations.relations().map(|(name, _)| name).collect();
            for Problem { at, error } in problems {
                let kind = ErrorKind::Namespace(quoted(error));
                let failure = (Some(places[at]), kind);
                self.push(Some(own), Some(names[at].as_str()), failure);
            }
        }
    }

    /// A namespace's relations, up to the next header or the end, checked
    /// whole, with the place of each, or `None` when the checks refuse it.
    /// Any error refuses the whole document, so a namespace returned beside
    /// one never leaves.
    fn relations(
        &mut self,
        namespace: &NamespaceName,
        theory: &Theory,
    ) -> Option<(Named, Vec<Place>)> {
        let mut relations = Vec::new();
        let mut places = Vec::new();
        // Bounded as the document's passes are.
        for _ in 0..self.lexemes.len() {
            if self.peek().token == Token::End || self.at_header() {
                break;
            }
            if !self.at_relation() {
                let failure = syntax(self.next(), RELATION);
                self.push(Some(namespace), None, failure);
                self.recover();
                continue;
            }
            // Read whole, so a malformed name is refused as one, not as the
            // relation its last part would name.
            let (lexeme, written) = self.run();
            let name = written.parse().unwrap_or_else(|error| {
                let failure = (Some(lexeme.place), ErrorKind::Name(error));
                self.push(Some(namespace), Some(written), failure);
                RelationName::new_unchecked(written)
            });
            let (mut constraints, mut failed) = if self.peek().token == Token::Less {
                self.next();
                self.constraints(namespace, &name, theory)
            } else {
                (Vec::new(), false)
            };
            let rewrite = if self.peek().token == Token::Colon {
                self.next();
                self.rewrite(namespace, &name)
            } else if self.at_boundary() {
                Some(Rewrite::This)
            } else {
                let failure = syntax(self.next(), AFTER_CONSTRAINTS);
                self.push(Some(namespace), Some(name.as_str()), failure);
                self.recover();
                None
            };
            failed |= rewrite.is_none();
            let rewrite = rewrite.unwrap_or(Rewrite::This);
            // A relation that fails is still declared, its constraints
            // agreeing with its rewrite, so the namespace reports nothing
            // more about it.
            if failed {
                if !reads_this(&rewrite) {
                    constraints.clear();
                } else if constraints.is_empty() {
                    constraints.push(Constraint::Identity);
                }
            }
            places.push(lexeme.place);
            relations.push((name, rewrite, constraints));
        }
        let names: Vec<_> = relations.iter().map(|(name, ..)| name.clone()).collect();
        match Namespace::with_depth(relations, self.depth) {
            Ok(relations) => Some((relations, places)),
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

    /// The constraints of relation `name`, whose `<` was just read, up to
    /// and with its `>`, and whether any failed. The first that fails is
    /// reported, and the rest of the list skipped, so one mistake is one
    /// error.
    fn constraints(
        &mut self,
        namespace: &NamespaceName,
        name: &RelationName,
        theory: &Theory,
    ) -> (Vec<NamedConstraint>, bool) {
        let mut constraints = Vec::new();
        // Each pass reads at least one lexeme or ends, so the lexemes bound
        // the passes.
        for _ in 0..self.lexemes.len() {
            let failure = match self.constraint(theory) {
                Ok(constraint) => {
                    constraints.push(constraint);
                    match self.peek().token {
                        Token::Comma => {
                            self.next();
                            continue;
                        }
                        Token::Greater => {
                            self.next();
                            return (constraints, false);
                        }
                        _ => syntax(self.peek(), AFTER_CONSTRAINT),
                    }
                }
                Err(failure) => failure,
            };
            self.push(Some(namespace), Some(name.as_str()), failure);
            break;
        }
        // Past the list's `>`, unless a relation, a header or the end comes
        // first.
        self.skip_until(|reader| reader.at_boundary() || reader.peek().token == Token::Greater);
        if self.peek().token == Token::Greater {
            self.next();
        }
        (constraints, true)
    }

    /// One constraint, its theory's name before its namespace name unless
    /// `..` comes first.
    fn constraint(&mut self, theory: &Theory) -> Result<NamedConstraint, Failure> {
        let Some((first, written)) = self.joined() else {
            return Err(syntax(self.peek(), CONSTRAINT));
        };
        theory
            .constraint(written)
            .map_err(|error| (Some(first.place), ErrorKind::Name(error)))
    }

    /// The rewrite of relation `name`, whose `:` was just read, or `None`
    /// when it fails, reported.
    fn rewrite(
        &mut self,
        namespace: &NamespaceName,
        name: &RelationName,
    ) -> Option<Rewrite<RelationName>> {
        // The rewrite runs to where the next relation or header starts, or
        // to the end.
        let start = self.at;
        self.recover();
        Parser::within(self.lexemes, start, self.at, self.depth)
            .parse()
            .map_err(|failure| self.push(Some(namespace), Some(name.as_str()), failure))
            .ok()
    }

    /// Every error in document order, capped at the setting, or `None`.
    fn finish(&mut self) -> Option<Vec<TdlError>> {
        if self.errors.is_empty() {
            return None;
        }
        let mut errors = std::mem::take(&mut self.errors);
        errors.sort_by_key(|error| error.place);
        if errors.len() > self.cap {
            errors.truncate(self.cap);
            errors.push(unplaced(ErrorKind::TooManyErrors(self.cap)));
        }
        Some(errors)
    }
}
