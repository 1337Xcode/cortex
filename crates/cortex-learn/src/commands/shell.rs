//! A shell command line read as the POSIX shell reads it, as far as telling which commands it runs
//! needs: words with their quotes removed, the operators that join commands (`&&`, `||`, `|`, `;`,
//! `&` and line breaks), `!`, and groups in parentheses or braces. Redirections are dropped with
//! their targets, comments and here-documents skipped. A line it cannot read for sure (a compound
//! command such as `if` or `for`, a function definition, an unclosed quote or group) is refused,
//! never guessed at.

/// A word as the shell passes it to a command, its quotes and escapes removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Word {
    /// The text.
    pub(super) text: String,
    /// Whether the shell passes the text as written: nothing in it expands (no `$`, backquote,
    /// glob character or leading `~` outside quotes that stop them).
    pub(super) is_literal: bool,
}

impl Word {
    /// The text, when the shell passes it as written.
    pub(super) fn literal(&self) -> Option<&str> {
        self.is_literal.then_some(self.text.as_str())
    }
}

/// A list of commands: and-or lists one after another, each ended by `;`, `&` or a line break.
#[derive(Debug, Default)]
pub(super) struct List {
    /// The and-or lists, in order.
    pub(super) items: Vec<AndOr>,
}

/// Pipelines joined by `&&` and `||`, run in the background when ended by `&`.
#[derive(Debug)]
pub(super) struct AndOr {
    /// The pipelines, in order.
    pub(super) pipelines: Vec<Pipeline>,
    /// What joins each pipeline to the next: one fewer than the pipelines.
    pub(super) joins: Vec<Join>,
    /// Whether `&` ended it, so that the shell does not wait for it.
    pub(super) is_background: bool,
}

/// What joins two pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Join {
    /// `&&`: the next runs when this one succeeded.
    And,
    /// `||`: the next runs when this one failed.
    Or,
}

/// Commands joined by `|`; its exit status is its last command's.
#[derive(Debug)]
pub(super) struct Pipeline {
    /// Whether `!` inverts its exit status.
    pub(super) is_negated: bool,
    /// The commands, in order.
    pub(super) commands: Vec<Command>,
}

/// One command of a pipeline.
#[derive(Debug)]
pub(super) enum Command {
    /// A command's words, the variable assignments before it included.
    Simple(Vec<Word>),
    /// A list in parentheses, run in a shell of its own, or in braces, run in this one.
    Group {
        /// The list inside.
        list: List,
        /// Whether it is in parentheses: a `cd` inside it does not move the rest of the line.
        is_subshell: bool,
    },
}

/// Why a line cannot be read.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Unreadable;

/// Reads `line` as a list of commands.
///
/// # Errors
///
/// The line holds what the reader does not follow, or is not a well-formed list.
pub(super) fn read(line: &str) -> Result<List, Unreadable> {
    let tokens = Lexer::new(line).tokens()?;
    let mut parser = Parser {
        tokens,
        at: 0,
        depth: 0,
    };
    let list = parser.list(Close::End)?;
    if parser.at == parser.tokens.len() {
        Ok(list)
    } else {
        Err(Unreadable)
    }
}

/// A piece of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// A word.
    Word(Word),
    /// `&&`.
    And,
    /// `||`.
    Or,
    /// `|` or `|&`.
    Pipe,
    /// `;`.
    Semicolon,
    /// `&`.
    Ampersand,
    /// A line break.
    LineBreak,
    /// `(`.
    Open,
    /// `)`.
    Close,
}

/// The most groups one inside another the reader follows: far more than a command line holds,
/// and few enough that reading a line built to nest deeper cannot exhaust the stack.
const MAX_GROUP_DEPTH: usize = 16;

/// Words the shell reads as the start or part of a compound command, which the reader refuses.
const COMPOUND_WORDS: &[&str] = &[
    "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "select", "function", "[[", "]]", "coproc",
];

/// Turns a line into tokens.
struct Lexer<'a> {
    /// The line's characters.
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    /// Here-documents begun on this line, by their delimiters, to skip at the next line break.
    here_documents: Vec<String>,
}

impl<'a> Lexer<'a> {
    /// A lexer at the start of `line`.
    fn new(line: &'a str) -> Self {
        Self {
            chars: line.chars().peekable(),
            here_documents: Vec::new(),
        }
    }

    /// Every token of the line.
    fn tokens(mut self) -> Result<Vec<Token>, Unreadable> {
        let mut tokens = Vec::new();
        while let Some(&next) = self.chars.peek() {
            match next {
                ' ' | '\t' => {
                    self.chars.next();
                }
                '\n' => {
                    self.chars.next();
                    tokens.push(Token::LineBreak);
                    self.skip_here_documents();
                }
                '#' => self.skip_comment(),
                ';' => {
                    self.chars.next();
                    if self.chars.peek() == Some(&';') {
                        return Err(Unreadable);
                    }
                    tokens.push(Token::Semicolon);
                }
                '&' | '|' | '(' | ')' | '<' | '>' => self.operator(&mut tokens)?,
                _ => self.word(&mut tokens)?,
            }
        }
        Ok(tokens)
    }

    /// Reads an operator or a redirection starting at `&`, `|`, `(`, `)`, `<` or `>`.
    fn operator(&mut self, tokens: &mut Vec<Token>) -> Result<(), Unreadable> {
        let Some(first) = self.chars.next() else {
            return Ok(());
        };
        match (first, self.chars.peek().copied()) {
            ('&', Some('&')) => {
                self.chars.next();
                tokens.push(Token::And);
            }
            ('&', Some('>')) => {
                // `&>` and `&>>`: both outputs to a file.
                self.chars.next();
                if self.chars.peek() == Some(&'>') {
                    self.chars.next();
                }
                self.redirection_target()?;
            }
            ('&', _) => tokens.push(Token::Ampersand),
            ('|', Some('|')) => {
                self.chars.next();
                tokens.push(Token::Or);
            }
            ('|', Some('&')) => {
                self.chars.next();
                tokens.push(Token::Pipe);
            }
            ('|', _) => tokens.push(Token::Pipe),
            ('(', _) => tokens.push(Token::Open),
            (')', _) => tokens.push(Token::Close),
            (_, _) => self.redirection(first)?,
        }
        Ok(())
    }

    /// Skips a redirection whose first character, `<` or `>`, was read, with its target.
    fn redirection(&mut self, first: char) -> Result<(), Unreadable> {
        if first == '<' && self.chars.peek() == Some(&'<') {
            self.chars.next();
            match self.chars.peek() {
                // A here-string: its word is the command's input.
                Some('<') => {
                    self.chars.next();
                }
                Some('-') => {
                    self.chars.next();
                    return self.here_document();
                }
                _ => return self.here_document(),
            }
        } else if matches!(self.chars.peek(), Some('>' | '&' | '|')) {
            self.chars.next();
        }
        self.redirection_target()
    }

    /// Reads a here-document's delimiter, to skip its lines at the next line break.
    fn here_document(&mut self) -> Result<(), Unreadable> {
        let delimiter = self.target_word()?;
        self.here_documents.push(delimiter.text);
        Ok(())
    }

    /// Skips a redirection's target: a word, or a file descriptor or `-` after `>&` and `<&`.
    fn redirection_target(&mut self) -> Result<(), Unreadable> {
        self.target_word().map(drop)
    }

    /// The word after a redirection operator.
    fn target_word(&mut self) -> Result<Word, Unreadable> {
        while matches!(self.chars.peek(), Some(' ' | '\t')) {
            self.chars.next();
        }
        let mut target = Vec::new();
        self.word(&mut target)?;
        match target.pop() {
            Some(Token::Word(word)) => Ok(word),
            _ => Err(Unreadable),
        }
    }

    /// Skips the lines of the here-documents begun on the line that just ended, each up to the
    /// line that is its delimiter (leading tabs aside).
    fn skip_here_documents(&mut self) {
        for delimiter in std::mem::take(&mut self.here_documents) {
            loop {
                let line: String = self.chars.by_ref().take_while(|&c| c != '\n').collect();
                if line.trim_start_matches('\t') == delimiter || self.chars.peek().is_none() {
                    break;
                }
            }
        }
    }

    /// Skips a comment, up to the line break.
    fn skip_comment(&mut self) {
        while self.chars.peek().is_some_and(|&c| c != '\n') {
            self.chars.next();
        }
    }

    /// Reads a word, or a redirection when the word is a file descriptor's number right before
    /// `<` or `>`.
    fn word(&mut self, tokens: &mut Vec<Token>) -> Result<(), Unreadable> {
        let mut word = Word {
            text: String::new(),
            is_literal: true,
        };
        let mut is_quoted = false;
        while let Some(&next) = self.chars.peek() {
            match next {
                ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' => break,
                '<' | '>' => {
                    if !is_quoted
                        && !word.text.is_empty()
                        && word.text.bytes().all(|b| b.is_ascii_digit())
                    {
                        self.chars.next();
                        return self.redirection(next);
                    }
                    break;
                }
                '\'' => {
                    self.chars.next();
                    is_quoted = true;
                    self.single_quoted(&mut word)?;
                }
                '"' => {
                    self.chars.next();
                    is_quoted = true;
                    self.double_quoted(&mut word)?;
                }
                '\\' => {
                    self.chars.next();
                    match self.chars.next() {
                        // A line continued.
                        Some('\n') => {}
                        Some(escaped) => word.text.push(escaped),
                        None => return Err(Unreadable),
                    }
                }
                '$' => {
                    self.chars.next();
                    word.is_literal = false;
                    self.expansion(&mut word)?;
                }
                '`' => {
                    self.chars.next();
                    word.is_literal = false;
                    self.backquoted(&mut word)?;
                }
                _ => {
                    self.chars.next();
                    if matches!(next, '*' | '?' | '[') || (next == '~' && word.text.is_empty()) {
                        word.is_literal = false;
                    }
                    word.text.push(next);
                }
            }
        }
        if !word.text.is_empty() || is_quoted {
            tokens.push(Token::Word(word));
        }
        Ok(())
    }

    /// Reads up to the closing `'`.
    fn single_quoted(&mut self, word: &mut Word) -> Result<(), Unreadable> {
        loop {
            match self.chars.next() {
                Some('\'') => return Ok(()),
                Some(c) => word.text.push(c),
                None => return Err(Unreadable),
            }
        }
    }

    /// Reads up to the closing `"`, where a backslash escapes only `$`, a backquote, `"`, `\` and
    /// a line break, and `$` and backquotes still expand.
    fn double_quoted(&mut self, word: &mut Word) -> Result<(), Unreadable> {
        loop {
            match self.chars.next() {
                Some('"') => return Ok(()),
                Some('\\') => match self.chars.next() {
                    Some('\n') => {}
                    Some(c @ ('$' | '`' | '"' | '\\')) => word.text.push(c),
                    Some(c) => {
                        word.text.push('\\');
                        word.text.push(c);
                    }
                    None => return Err(Unreadable),
                },
                Some('$') => {
                    word.is_literal = false;
                    self.expansion(word)?;
                }
                Some('`') => {
                    word.is_literal = false;
                    self.backquoted(word)?;
                }
                Some(c) => word.text.push(c),
                None => return Err(Unreadable),
            }
        }
    }

    /// Reads what follows a `$`: a name, `{...}`, `(...)` or `((...))`, kept as written.
    fn expansion(&mut self, word: &mut Word) -> Result<(), Unreadable> {
        word.text.push('$');
        match self.chars.peek() {
            Some('{') => self.balanced(word, '{', '}'),
            Some('(') => self.balanced(word, '(', ')'),
            Some('\'') => {
                // `$'...'`: quoted text with escapes, kept as written.
                self.chars.next();
                self.single_quoted(word)
            }
            _ => {
                while let Some(&c) = self.chars.peek() {
                    if !(c.is_ascii_alphanumeric() || c == '_') {
                        break;
                    }
                    word.text.push(c);
                    self.chars.next();
                }
                Ok(())
            }
        }
    }

    /// Reads from an opening `open` to its matching `close`, quotes inside skipped whole, kept as
    /// written.
    fn balanced(&mut self, word: &mut Word, open: char, close: char) -> Result<(), Unreadable> {
        let mut depth = 0_usize;
        while let Some(c) = self.chars.next() {
            word.text.push(c);
            match c {
                '\\' => {
                    if let Some(escaped) = self.chars.next() {
                        word.text.push(escaped);
                    }
                }
                '\'' | '"' => {
                    for inner in self.chars.by_ref() {
                        word.text.push(inner);
                        if inner == c {
                            break;
                        }
                    }
                }
                _ if c == open => depth += 1,
                _ if c == close => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(Unreadable)
    }

    /// Reads up to the closing backquote, kept as written.
    fn backquoted(&mut self, word: &mut Word) -> Result<(), Unreadable> {
        word.text.push('`');
        while let Some(c) = self.chars.next() {
            word.text.push(c);
            match c {
                '\\' => {
                    if let Some(escaped) = self.chars.next() {
                        word.text.push(escaped);
                    }
                }
                '`' => return Ok(()),
                _ => {}
            }
        }
        Err(Unreadable)
    }
}

/// What ends a list being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Close {
    /// The end of the line.
    End,
    /// `)`.
    Parenthesis,
    /// The word `}`.
    Brace,
}

/// Reads tokens into a list.
struct Parser {
    /// The line's tokens.
    tokens: Vec<Token>,
    /// The next token to read.
    at: usize,
    /// How many groups the token to read is inside.
    depth: usize,
}

impl Parser {
    /// The token to read next, if any.
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    /// Whether the next token ends a list closed by `close`.
    fn is_close(&self, close: Close) -> bool {
        match (self.peek(), close) {
            (None, _) | (Some(Token::Close), Close::Parenthesis) => true,
            (Some(Token::Word(word)), Close::Brace) => word.is_literal && word.text == "}",
            _ => false,
        }
    }

    /// Skips line breaks.
    fn skip_line_breaks(&mut self) {
        while self.peek() == Some(&Token::LineBreak) {
            self.at += 1;
        }
    }

    /// A list, up to what `close` names (not read).
    fn list(&mut self, close: Close) -> Result<List, Unreadable> {
        let mut list = List::default();
        loop {
            while matches!(self.peek(), Some(Token::LineBreak | Token::Semicolon)) {
                self.at += 1;
            }
            if self.is_close(close) {
                return Ok(list);
            }
            let mut item = self.and_or(close)?;
            match self.peek() {
                Some(Token::Ampersand) => {
                    self.at += 1;
                    item.is_background = true;
                }
                Some(Token::Semicolon | Token::LineBreak) => self.at += 1,
                _ if self.is_close(close) => {}
                _ => return Err(Unreadable),
            }
            list.items.push(item);
        }
    }

    /// Pipelines joined by `&&` and `||`.
    fn and_or(&mut self, close: Close) -> Result<AndOr, Unreadable> {
        let mut pipelines = vec![self.pipeline(close)?];
        let mut joins = Vec::new();
        loop {
            let join = match self.peek() {
                Some(Token::And) => Join::And,
                Some(Token::Or) => Join::Or,
                _ => break,
            };
            self.at += 1;
            self.skip_line_breaks();
            joins.push(join);
            pipelines.push(self.pipeline(close)?);
        }
        Ok(AndOr {
            pipelines,
            joins,
            is_background: false,
        })
    }

    /// Commands joined by `|`, `!` first when it inverts them.
    fn pipeline(&mut self, close: Close) -> Result<Pipeline, Unreadable> {
        let is_negated =
            matches!(self.peek(), Some(Token::Word(word)) if word.is_literal && word.text == "!");
        if is_negated {
            self.at += 1;
        }
        let mut commands = vec![self.command(close)?];
        while self.peek() == Some(&Token::Pipe) {
            self.at += 1;
            self.skip_line_breaks();
            commands.push(self.command(close)?);
        }
        Ok(Pipeline {
            is_negated,
            commands,
        })
    }

    /// One command: a group, or a simple command's words.
    fn command(&mut self, close: Close) -> Result<Command, Unreadable> {
        match self.peek() {
            Some(Token::Open) => {
                self.at += 1;
                let list = self.group(Close::Parenthesis)?;
                Ok(Command::Group {
                    list,
                    is_subshell: true,
                })
            }
            Some(Token::Word(word)) if word.is_literal && word.text == "{" => {
                self.at += 1;
                let list = self.group(Close::Brace)?;
                Ok(Command::Group {
                    list,
                    is_subshell: false,
                })
            }
            _ => self.simple(close),
        }
    }

    /// The list of a group up to `close`, read, which must hold a command.
    fn group(&mut self, close: Close) -> Result<List, Unreadable> {
        if self.depth >= MAX_GROUP_DEPTH {
            return Err(Unreadable);
        }
        self.depth += 1;
        let list = self.list(close)?;
        self.depth -= 1;
        if list.items.is_empty() || self.peek().is_none() {
            return Err(Unreadable);
        }
        self.at += 1;
        Ok(list)
    }

    /// A simple command's words, which must hold one.
    fn simple(&mut self, close: Close) -> Result<Command, Unreadable> {
        let mut words = Vec::new();
        while let Some(Token::Word(word)) = self.peek() {
            if close == Close::Brace && word.is_literal && word.text == "}" {
                break;
            }
            if words.is_empty() && word.is_literal && COMPOUND_WORDS.contains(&word.text.as_str()) {
                return Err(Unreadable);
            }
            words.push(word.clone());
            self.at += 1;
        }
        // A function definition, `name() ...`, or an empty command.
        if words.is_empty() || self.peek() == Some(&Token::Open) {
            return Err(Unreadable);
        }
        Ok(Command::Simple(words))
    }
}
