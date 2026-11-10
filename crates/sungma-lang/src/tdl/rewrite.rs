//! TDL's tokens and its rewrite grammar: the lexer every document is read
//! with, the parser for one relation's rewrite, and the kinds of problem a
//! document is refused with.

use thiserror::Error;

use sungma::{
    name::{NameError, RelationName, quote},
    namespace::NamespaceError,
    non_empty::NonEmpty,
    rewrite::Rewrite,
};

#[derive(Debug, Error)]
pub enum ErrorKind {
    #[error("a document doesn't start with a byte-order mark")]
    ByteOrderMark,
    #[error("a document declares no namespace")]
    NoNamespace,
    /// A document given where one namespace is expected, such as a
    /// namespace write or a stored version, that declares more.
    #[error("a document here declares one namespace, not {0}")]
    ManyNamespaces(usize),
    #[error("theory '{0}' declares no namespace")]
    EmptyTheory(String),
    #[error("theory '{0}' isn't one name: a theory name has no dot")]
    DottedTheory(String),
    #[error(transparent)]
    Name(NameError),
    #[error("expected {expected}, found {found}")]
    Syntax {
        found: String,
        expected: &'static str,
    },
    /// Parentheses grouped deeper than the depth limit, which it carries.
    #[error("parentheses nest more than {0} deep")]
    TooManyParentheses(usize),
    #[error(transparent)]
    Namespace(NamespaceError),
    #[error("too many errors; the first {0} are reported")]
    TooManyErrors(usize),
    #[error("namespace '{0}' is declared twice")]
    DuplicateNamespace(String),
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
    /// `<`, which opens a relation's constraints.
    Less,
    /// `>`, which closes them.
    Greater,
    /// A character no token starts with, such as the `@`, `#` and `.` that
    /// only appear inside a name or a constraint.
    Unknown,
    End,
}

/// Where a lexeme starts: its line, and its column counting characters from
/// 1.
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

/// A document's lexemes, ending in [`Token::End`]. Whitespace, line breaks
/// included, only separates them, and `//` starts a comment to the end of
/// its line.
pub(crate) fn lex(text: &str) -> Vec<Lexeme<'_>> {
    let mut lexemes = Vec::new();
    let mut chars = text.char_indices().peekable();
    let (mut line, mut column) = (1, 0);
    while let Some((start, c)) = chars.next() {
        column += 1;
        let place = Place { line, column };
        let token = match c {
            '\n' => {
                (line, column) = (line + 1, 0);
                continue;
            }
            ' ' | '\t' | '\r' => continue,
            '/' if matches!(chars.peek(), Some((_, '/'))) => {
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
            '<' => Token::Less,
            '>' => Token::Greater,
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

/// A node and how many levels deep it nests: a leaf is one level, and a
/// run of `|` or `&`, or one `!`, adds one.
type Leveled = (Node, usize);

/// Recursive descent over the rewrite grammar, one function per level.
///
/// The parser holds the tree it builds to the depth limit as it builds it,
/// and its parentheses too, so no rewrite, however long, builds a tree
/// deeper than the limit or recurses deeper than it.
pub(crate) struct Parser<'l, 'a> {
    lexemes: &'l [Lexeme<'a>],
    at: usize,
    /// Where the rewrite ends: the parser reads the lexeme here as the end,
    /// whatever it is.
    end: usize,
    groups: usize,
    depth: usize,
}

impl<'l, 'a> Parser<'l, 'a> {
    /// A parser for the rewrite from lexeme `start` up to lexeme `end`,
    /// nesting at most `depth` levels and grouping at most `depth`
    /// parentheses.
    pub(crate) fn within(
        lexemes: &'l [Lexeme<'a>],
        start: usize,
        end: usize,
        depth: usize,
    ) -> Self {
        Self {
            lexemes,
            at: start,
            end,
            groups: 0,
            depth,
        }
    }

    /// A whole rewrite, then the end.
    pub(crate) fn parse(mut self) -> Result<Rewrite<RelationName>, Failure> {
        let (rewrite, _) = self.union()?;
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

    /// `level`, refused at `place` when it's past the depth limit.
    fn level(&self, level: usize, place: Place) -> Result<usize, Failure> {
        if level > self.depth {
            let kind = ErrorKind::Namespace(NamespaceError::TooDeep(self.depth));
            return Err((Some(place), kind));
        }
        Ok(level)
    }

    fn union(&mut self) -> Result<Leveled, Failure> {
        self.run(Token::Union, Self::intersection, Rewrite::Union)
    }

    fn intersection(&mut self) -> Result<Leveled, Failure> {
        self.run(Token::Intersection, Self::exclusion, Rewrite::Intersection)
    }

    /// A run of one operator, as one node with an operand for each term.
    fn run(
        &mut self,
        operator: Token,
        operand: fn(&mut Self) -> Result<Leveled, Failure>,
        node: fn(NonEmpty<Rewrite<RelationName>>) -> Rewrite<RelationName>,
    ) -> Result<Leveled, Failure> {
        let (first, mut deepest) = operand(self)?;
        let place = self.peek().place;
        if self.peek().token != operator {
            return Ok((first, deepest));
        }
        let mut rest = Vec::new();
        while self.peek().token == operator {
            self.next();
            let (next, level) = operand(self)?;
            deepest = deepest.max(level);
            rest.push(next);
        }
        let level = self.level(deepest + 1, place)?;
        Ok((node(NonEmpty::with_rest(first, rest)), level))
    }

    fn exclusion(&mut self) -> Result<Leveled, Failure> {
        let (mut base, mut level) = self.term()?;
        while self.peek().token == Token::Exclusion {
            let operator = self.next();
            let (excluded, beside) = self.term()?;
            level = self.level(level.max(beside) + 1, operator.place)?;
            base = Rewrite::Exclusion(Box::new(base), Box::new(excluded));
        }
        Ok((base, level))
    }

    fn term(&mut self) -> Result<Leveled, Failure> {
        let lexeme = self.next();
        let leaf = match lexeme.token {
            Token::This => Rewrite::This,
            Token::Name => Rewrite::Computed(relation(lexeme)?),
            Token::Open if self.peek_is_fact_to() => self.fact_to()?,
            Token::Open => {
                self.groups += 1;
                if self.groups > self.depth {
                    return Err((
                        Some(lexeme.place),
                        ErrorKind::TooManyParentheses(self.depth),
                    ));
                }
                let node = self.union()?;
                self.close()?;
                self.groups -= 1;
                return Ok(node);
            }
            _ => return Err(syntax(lexeme, TERM)),
        };
        Ok((leaf, self.level(1, lexeme.place)?))
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

/// Whether a rewrite reads `this`.
pub(crate) fn reads_this<R>(rewrite: &Rewrite<R>) -> bool {
    match rewrite {
        Rewrite::This => true,
        Rewrite::Computed(_) | Rewrite::FactTo { .. } => false,
        Rewrite::Union(operands) | Rewrite::Intersection(operands) => {
            operands.iter().any(reads_this)
        }
        Rewrite::Exclusion(base, excluded) => reads_this(base) || reads_this(excluded),
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
        NamespaceError::Unconstrained(relation) => NamespaceError::Unconstrained(quote(&relation)),
        NamespaceError::ConstrainedComputed(relation) => {
            NamespaceError::ConstrainedComputed(quote(&relation))
        }
        NamespaceError::DuplicateConstraint {
            relation,
            constraint,
        } => NamespaceError::DuplicateConstraint {
            relation: quote(&relation),
            constraint: quote(&constraint),
        },
        NamespaceError::UndeclaredConstraint {
            relation,
            constraint,
        } => NamespaceError::UndeclaredConstraint {
            relation: quote(&relation),
            constraint: quote(&constraint),
        },
        NamespaceError::UndeclaredFollowed {
            relation,
            namespace,
            followed,
        } => NamespaceError::UndeclaredFollowed {
            relation: quote(&relation),
            namespace: quote(&namespace),
            followed: quote(&followed),
        },
        NamespaceError::TooDeep(depth) => NamespaceError::TooDeep(depth),
    }
}
