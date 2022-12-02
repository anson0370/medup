use std::fmt;
use std::fmt::Debug;

// Ast represents the abstract syntax tree of the markdown file, it structurally represents the entire file.
pub struct Ast {
    lines: Vec<Line>,
}

impl Ast {
    pub fn new() -> Ast {
        Ast { lines: Vec::new() }
    }

    pub fn push(&mut self, line: Line) {
        self.lines.push(line);
    }
}

impl Default for Ast {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for Ast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = String::new();
        for line in &self.lines {
            for t in &line.tokens {
                let s = format!("<{}, {}, {:?}> ", t.value, t.line_num, t.kind);
                debug.push_str(&s);
            }
            debug.push('\n');
        }
        writeln!(f, "{}", debug)
    }
}

// Line is a line of the markdown file, it be parsed into some tokens.
pub struct Line {
    tokens: Vec<Token>,
    kind: LineKind,
}

enum LineKind {
    Unknow,
    Blank,
    Title,
    Plain,
}

// Token is a part of the line, the parser will parse the line into some tokens.
struct Token {
    value: String,
    kind: TokenKind,
    line_num: i32,
}

#[derive(PartialEq, Debug)]
enum TokenKind {
    TitleMark,
    DisorderMark,
    DividingMark,
    QuoteMark,
    Title,
    DisorderListItem,
    Quote,
    BlankLine,
    Plain,
}

impl Line {
    // parses one line text into Line that contains multi tokens.
    pub fn parse(ln: i32, line: String) -> Line {
        let mut statem = StateMachine::new(&line);

        for (current, ch) in line.chars().enumerate() {
            let finished = statem.process(current, ch);
            if finished {
                break;
            }
        }
        let mut tokens = statem.close();
        for mut t in &mut tokens {
            t.line_num = ln;
        }
        let kind = match tokens.first() {
            None => LineKind::Unknow,
            Some(t) => {
                if t.kind == TokenKind::BlankLine {
                    LineKind::Blank
                } else if t.kind == TokenKind::TitleMark {
                    LineKind::Title
                } else if t.kind == TokenKind::Plain {
                    LineKind::Plain
                } else {
                    LineKind::Unknow
                }
            }
        };

        Line { tokens, kind }
    }
}

// StateMaching represents the current state of the parser.
struct StateMachine<'a> {
    state: State,
    pointer: usize,
    times: i32,
    tokens: Vec<Token>,
    text: &'a String,
}

#[derive(PartialEq)]
enum State {
    Begin,
    CheckMark,
    Title,
    DisorderedList,
    Quote,
    CheckDividing,
    Plain,
    Finished,
}

impl<'a> StateMachine<'a> {
    fn new(text: &'a String) -> Self {
        StateMachine {
            state: State::Begin,
            pointer: 0,
            times: 0,
            text,
            tokens: Vec::new(),
        }
    }

    fn process(&mut self, current: usize, ch: char) -> bool {
        self.times = 1;
        while self.times > 0 {
            self.times -= 1;

            let t = match self.state {
                State::Begin => self.skip_begin_whitespaces(current, ch),
                State::CheckMark => self.check_mark(current),
                State::Title => self.parse_title(current, ch),
                State::DisorderedList => self.parse_disordered_list(current, ch),
                State::CheckDividing => None,
                State::Quote => self.parse_quote(current, ch),
                State::Plain => self.parse_plain(current, ch),
                State::Finished => None,
            };

            if let Some(token) = t {
                self.tokens.push(token);
            }
        }

        self.state == State::Finished
    }

    fn close(self) -> Vec<Token> {
        self.tokens
    }

    // skip all whitespace characters at the beginning of the line,
    // or generate a blank line token if the line contains only whitespace characters.
    fn skip_begin_whitespaces(&mut self, current: usize, ch: char) -> Option<Token> {
        if ch.is_whitespace() {
            if ch == '\n' {
                self.state = State::Finished;
                self.pointer = 0;
                return Some(Token {
                    value: "".to_string(),
                    kind: TokenKind::BlankLine,
                    line_num: 0,
                });
            }
        } else {
            self.state = State::CheckMark;
            self.pointer = current;
        }
        None
    }

    fn find_word(&self, current: usize) -> Option<&str> {
        let ch = self.text.chars().nth(current).unwrap(); // note: unwrap, it is safe here.
        if ch.is_whitespace() {
            Some(&self.text[self.pointer..current])
        } else {
            None
        }
    }

    // parse the first word in the line as the mark token
    fn check_mark(&mut self, current: usize) -> Option<Token> {
        let first_word = self.find_word(current)?;

        let (pointer, state, token) = match first_word {
            // title
            "#" | "##" | "###" | "####" | "#####" => (
                current,
                State::Title,
                Some(Token {
                    value: first_word.to_string(),
                    kind: TokenKind::TitleMark,
                    line_num: 0,
                }),
            ),

            // disordered list
            "*" | "-" | "+" => (
                current,
                State::DisorderedList,
                Some(Token {
                    value: first_word.to_string(),
                    kind: TokenKind::DisorderMark,
                    line_num: 0,
                }),
            ),

            // dividing line
            // TODO: support more dividing line marksu
            "***" | "---" | "___" => (
                current,
                State::CheckDividing,
                Some(Token {
                    value: first_word.to_string(),
                    kind: TokenKind::DividingMark,
                    line_num: 0,
                }),
            ),

            // quote
            ">" => (
                current,
                State::Quote,
                Some(Token {
                    value: first_word.to_string(),
                    kind: TokenKind::QuoteMark,
                    line_num: 0,
                }),
            ),

            // plain (as no mark)
            _ => {
                self.times += 1;
                // don't change the pointer, because the first word is not a mark.
                (self.pointer, State::Plain, None)
            }
        };

        self.state = state;
        self.pointer = pointer;
        token
    }

    // parse the rest of the line as title token
    fn parse_title(&mut self, current: usize, ch: char) -> Option<Token> {
        // skip all whitespace characters after the mark token.
        if ch.is_whitespace() {
            return None;
        }
        self.state = State::Finished;
        let rest = &self.text[current..];
        Some(Token {
            value: rest.trim_end_matches('\n').to_string(),
            kind: TokenKind::Title,
            line_num: 0,
        })
    }

    // parse the rest of the line as the disordered list token.
    fn parse_disordered_list(&mut self, current: usize, ch: char) -> Option<Token> {
        // skip all whitespace characters after the mark token.
        if ch.is_whitespace() {
            return None;
        }
        self.state = State::Finished;
        let rest = &self.text[current..];
        Some(Token {
            value: rest.trim_end_matches('\n').to_string(),
            kind: TokenKind::DisorderListItem,
            line_num: 0,
        })
    }

    // parse the rest of the line as the quote token.
    fn parse_quote(&mut self, current: usize, ch: char) -> Option<Token> {
        // skip all whitespace characters after the mark token.
        if ch.is_whitespace() {
            return None;
        }
        self.state = State::Finished;
        let rest = &self.text[current..];
        Some(Token {
            value: rest.trim_end_matches('\n').to_string(),
            kind: TokenKind::Quote,
            line_num: 0,
        })
    }

    // parse the line as the plain token.
    fn parse_plain(&mut self, _current: usize, _ch: char) -> Option<Token> {
        self.state = State::Finished;
        let content = &self.text[self.pointer..];
        Some(Token {
            value: content.trim_end_matches('\n').to_string(),
            kind: TokenKind::Plain,
            line_num: 0,
        })
    }
}
