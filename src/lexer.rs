use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
};

use crate::stack;

use email_address::EmailAddress;
use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use url::Url;

// This regex is used to match a string with double quotes("") or single quotes('')
lazy_static! {
    static ref D_QUOTED_STRING_RE: Regex = Regex::new("^\"([^\"\\\\]|\\\\.)*\"$").unwrap();
    static ref S_QUOTED_STRING_RE: Regex = Regex::new("^\'([^\'\\\\]|\\\\.)*\'$").unwrap();
}

const ESCAPE_CHARS: &str = ":*_`#+-.![]()<>\\";

#[derive(PartialEq, Debug)]
enum State {
    Begin,
    Mark(usize),
    Inline(usize),
    Finished,
}

#[derive(PartialEq, Clone, Copy)]
enum InlineState {
    Skip,
    Finished,
    Normal,
    // means *, usize is start position in line
    Continuous(usize),
    // means !, usize is the index of '!'
    ImgBegin(usize),
    // means [, (usize, usize) is the index of ('!', '[')
    ImgNameBegin(usize, usize),
    // means [, usize is the index of '['
    LinkNameBegin(usize),
    // means ], (usize, usize, usize) is the index of ('!', '[', ']')
    NameEnd(Option<usize>, usize, usize),
    // means [, (usize, usize, usize) is the index of ('[', ']', '[')
    RefLink(usize, usize, usize),
    // means :, (usize, usize, usize) is the index of ('[', ']', ':')
    RefLinkDef(usize, usize, usize),
    // means (, (usize, usize, usize, usize) is the index of ('!', '[', ']', '(')
    Location(Option<usize>, usize, usize, usize),
    // means <, usize is the index of '<'
    QuickLink(usize),
}

// Lexer is a lexical analyzer that parses lines of text into multiple tokens.
pub(crate) struct Lexer<'lexer> {
    state: State,
    line_text: &'lexer str,
}

impl<'lexer> Lexer<'lexer> {
    pub(crate) fn new(text: &'lexer str) -> Self {
        Lexer {
            state: State::Begin,
            line_text: text,
        }
    }

    pub(crate) fn split(mut self) -> Vec<Token> {
        let mut buff = vec![];

        let iter = self.line_text.chars().enumerate().peekable();
        for (ix, curr) in iter {
            match self.state {
                State::Begin => {
                    if !curr.is_whitespace() {
                        let s = self.slice(0, ix);
                        if !s.is_empty() {
                            buff.push(Token::new(s, TokenKind::WhiteSpace));
                        }
                        self.goto(State::Mark(ix));
                    } else {
                        // the end of iterator
                        if curr == '\n' {
                            buff.push(Token::new(self.slice(0, ix), TokenKind::BlankLine));
                        } else {
                            // keep this state
                        }
                    }
                }

                State::Mark(begin) => {
                    // find the first word
                    let first_word = if curr.is_whitespace() {
                        // the current character is white space
                        self.slice_str(begin, ix)
                    } else {
                        continue;
                    };

                    if let Some(m) = self.extract_mark(first_word) {
                        match m.kind() {
                            TokenKind::CodeBlockMark => self.goto(State::Inline(begin + 3)),
                            TokenKind::DividingMark => self.goto(State::Finished),
                            _ => self.goto(State::Inline(ix + 1)),
                        }
                        buff.push(m);
                    } else {
                        // normal text
                        self.goto(State::Inline(begin));
                    }
                }
                State::Inline(_) => {
                    break;
                }
                State::Finished => {
                    break;
                }
            };
        }

        if let State::Inline(begin) = self.state {
            let rest = self.slice_rest(begin);

            for t in Self::split_inline(rest)
                .into_iter()
                .filter(|t| !t.value().is_empty())
            {
                buff.push(t);
            }
        }
        buff
    }

    // Parse the first word in the line as the mark token
    fn extract_mark(&self, first_word: &str) -> Option<Token> {
        let first_word_chars: Vec<char> = first_word.chars().collect();

        match first_word_chars[..] {
            // Title
            ['#'] | ['#', '#'] | ['#', '#', '#'] | ['#', '#', '#', '#'] => {
                Some(Token::new(first_word.to_string(), TokenKind::TitleMark))
            }

            // Ordered List
            [n1, '.'] if ('1'..='9').contains(&n1) => {
                Some(Token::new(first_word.to_string(), TokenKind::OrderedMark))
            }
            [n1, n2, '.'] if ('1'..='9').contains(&n1) && ('0'..='9').contains(&n2) => {
                Some(Token::new(first_word.to_string(), TokenKind::OrderedMark))
            }
            [n1, n2, n3, '.']
                if ('1'..='9').contains(&n1)
                    && ('0'..='9').contains(&n2)
                    && ('0'..='9').contains(&n3) =>
            {
                Some(Token::new(first_word.to_string(), TokenKind::OrderedMark))
            }

            // Quote
            ['>'] => Some(Token::new(first_word.to_string(), TokenKind::QuoteMark)),

            // Code Block
            // .e.g:
            //      ```rust
            //      ``` rust
            ['`', '`', '`', ..] => Some(Token::new("```".to_string(), TokenKind::CodeBlockMark)),

            // Unordered List
            ['+'] => Some(Token::new(first_word.to_string(), TokenKind::UnorderedMark)),

            // Unordered List or Dividing Line
            ['*'] | ['-'] => {
                if Self::is_dividing(self.line_text) {
                    // Here is a dividing line, not list
                    return Some(Token::new(
                        self.line_text.trim_end_matches('\n').to_string(),
                        TokenKind::DividingMark,
                    ));
                }
                // Here is a unordered list
                Some(Token::new(first_word.to_string(), TokenKind::UnorderedMark))
            }

            // Dividing Line
            ['*', ..] | ['-', ..] | ['_', ..] => {
                if Self::is_dividing(self.line_text) {
                    Some(Token::new(
                        self.line_text.trim_end_matches('\n').to_string(),
                        TokenKind::DividingMark,
                    ))
                } else {
                    None
                }
            }

            // normal (as no mark)
            _ => {
                // don't change the unparsed pointer, because the first word is not a mark.
                None
            }
        }
    }

    // Parse inline syntax, include bold, image and link etc.
    fn split_inline(content: &str) -> Vec<Token> {
        let mut last = 0;

        let mut buff: Vec<Token> = Vec::new();
        let mut state = InlineState::Normal;

        let mut content_iter = content.chars().enumerate().peekable();
        while let Some((ix, ch)) = content_iter.next() {
            match (state, ch) {
                (_, '\n') => {
                    // end of the line
                    let s = utf8_slice::slice(content, last, ix)
                        .trim_end()
                        .trim_end_matches("<br>")
                        .to_string();
                    if !s.is_empty() {
                        buff.push(Token::new(s, TokenKind::Text));
                    }
                    break;
                }
                (_, '\\') => {
                    let next = content_iter.peek().map(|(_, n)| *n).unwrap_or('x');
                    if ESCAPE_CHARS.contains(next) {
                        // need to skip the next character
                        state = InlineState::Skip;

                        let s = utf8_slice::slice(content, last, ix);
                        if !s.is_empty() {
                            buff.push(Token::new(s.to_string(), TokenKind::Text));
                        }
                        last = ix + 1; // drop the character: '\'
                    }
                }
                (InlineState::Skip, _) => {
                    state = InlineState::Normal;
                }
                (InlineState::Normal, _) => match ch {
                    '*' | '_' | '`' => {
                        // the part of normal text before mark.
                        let s = utf8_slice::slice(content, last, ix);
                        if !s.is_empty() {
                            buff.push(Token::new(s.to_string(), TokenKind::Text));
                        }

                        last = ix;

                        if content_iter.peek().map(|(_, n)| *n).unwrap_or(' ') == ch {
                            state = InlineState::Continuous(ix);
                        } else {
                            let s = utf8_slice::slice(content, ix, ix + 1);
                            last = ix + 1;
                            let k = match ch {
                                '*' => TokenKind::Star,
                                '_' => TokenKind::UnderLine,
                                '`' => TokenKind::BackTick,
                                _ => unreachable!(),
                            };
                            buff.push(Token::new(s.to_string(), k));
                        }
                    }
                    '!' => state = InlineState::ImgBegin(ix),
                    '[' => state = InlineState::LinkNameBegin(ix),
                    '<' => state = InlineState::QuickLink(ix),
                    _ => (),
                },
                (InlineState::ImgBegin(begin), _) => match ch {
                    '[' => state = InlineState::ImgNameBegin(begin, ix),
                    '!' => state = InlineState::ImgBegin(ix),
                    _ => state = InlineState::Normal,
                },
                (InlineState::ImgNameBegin(b1, b2), _) => {
                    if ch == ']' {
                        state = InlineState::NameEnd(Some(b1), b2, ix);
                    }
                }
                (InlineState::LinkNameBegin(begin), _) => match ch {
                    ']' => state = InlineState::NameEnd(None, begin, ix),
                    '[' => state = InlineState::LinkNameBegin(ix),
                    _ => (),
                },
                (InlineState::NameEnd(b1, b2, b3), _) => match ch {
                    '(' => state = InlineState::Location(b1, b2, b3, ix),
                    ']' => state = InlineState::NameEnd(b1, b2, ix),
                    '[' => state = InlineState::RefLink(b2, b3, ix),
                    ':' => state = InlineState::RefLinkDef(b2, b3, ix),
                    _ => state = InlineState::Normal,
                },
                (InlineState::RefLink(b1, b2, b3), _) => {
                    if ch == ']' {
                        let s = utf8_slice::slice(content, last, b1);
                        if !s.is_empty() {
                            buff.push(Token::new(s.to_string(), TokenKind::Text));
                        }

                        let s = utf8_slice::slice(content, b1, ix + 1);
                        let s1 = utf8_slice::slice(content, b1 + 1, b2);
                        let s2 = utf8_slice::slice(content, b3 + 1, ix);
                        let t = Self::split_generic_link_details(s, s1, s2, TokenKind::RefLink);
                        buff.push(t);

                        last = ix + 1;
                        state = InlineState::Normal;
                    }
                }
                (InlineState::RefLinkDef(b1, b2, _b3), _) => {
                    let s = utf8_slice::from(content, last).trim_end_matches('\n');
                    let s1 = utf8_slice::slice(content, b1 + 1, b2);
                    let s2 = utf8_slice::from(content, ix).trim_end_matches('\n');
                    let t = Self::split_generic_link_details(s, s1, s2, TokenKind::RefLinkDef);
                    buff.push(t);

                    state = InlineState::Finished;
                }
                (InlineState::Location(b1, b2, b3, b4), _) => {
                    if ch == ')' {
                        // when found ')', this means that we found a valid image or link.
                        let begin = b1.unwrap_or(b2);
                        // the part of normal text before '![]()' or '[]()' mark.
                        let s = utf8_slice::slice(content, last, begin);
                        if !s.is_empty() {
                            buff.push(Token::new(s.to_string(), TokenKind::Text));
                        }
                        // '![]()' or '[]()' mark
                        let s = utf8_slice::slice(content, begin, ix + 1);
                        let s1 = utf8_slice::slice(content, b2 + 1, b3); // s1 in []
                        let s2 = utf8_slice::slice(content, b4 + 1, ix); // s2 in ()
                        let t = if b1.is_some() {
                            // image
                            Self::split_generic_link_details(s, s1, s2, TokenKind::Image)
                        } else {
                            // link
                            Self::split_generic_link_details(s, s1, s2, TokenKind::Link)
                        };
                        buff.push(t);

                        last = ix + 1;
                        state = InlineState::Normal;
                    }
                }
                (InlineState::QuickLink(begin), _) => {
                    if ch.is_whitespace() {
                        let s = utf8_slice::slice(content, begin + 1, ix).trim();
                        if !s.is_empty() && !Self::is_url(s) && !Self::is_email(s) {
                            state = InlineState::Normal;
                        }
                    }
                    if ch == '>' {
                        let link = utf8_slice::slice(content, begin + 1, ix).trim();
                        if Self::is_url(link) || Self::is_email(link) {
                            let before = utf8_slice::slice(content, last, begin);
                            if !before.is_empty() {
                                buff.push(Token::new(before.to_string(), TokenKind::Text));
                            }

                            let s = utf8_slice::slice(content, begin, ix + 1);
                            let t = Self::split_generic_link_details(
                                s,
                                link,
                                link,
                                TokenKind::QuickLink,
                            );
                            buff.push(t);

                            last = ix + 1;
                            state = InlineState::Normal;
                        } else {
                            state = InlineState::Normal;
                        }
                    }
                }
                (InlineState::Continuous(begin), _) => {
                    if *content_iter.peek().map(|(_, n)| n).unwrap_or(&' ') != ch {
                        let s = utf8_slice::slice(content, begin, ix + 1);
                        let k = match ch {
                            '*' => TokenKind::Star,
                            '_' => TokenKind::UnderLine,
                            '`' => TokenKind::BackTick,
                            _ => unreachable!(),
                        };
                        buff.push(Token::new(s.to_string(), k));

                        last = ix + 1;
                        state = InlineState::Normal;
                    }
                }
                (InlineState::Finished, _) => {
                    break;
                }
            }
        }
        if Self::has_br(content) {
            buff.push(Token::new("<br>".to_string(), TokenKind::LineBreak));
        }
        Self::tidy(&mut buff);
        buff
    }

    fn split_generic_link_details(s: &str, s1: &str, s2: &str, kind: TokenKind) -> Token {
        let s2 = s2.trim();
        let fields: Vec<&str> = s2.splitn(2, [' ', '\t']).collect();
        let (kind, location, title) = match fields.len().cmp(&2) {
            Ordering::Less => (kind, s2, ""),
            Ordering::Equal => {
                if Self::is_quoted_string(fields[1]) {
                    (kind, fields[0], fields[1])
                } else {
                    (TokenKind::Text, "", "")
                }
            }
            Ordering::Greater => unreachable!(),
        };

        let mut t = Token::new(s.to_string(), kind);
        let rf = &mut t;
        match kind {
            TokenKind::Image | TokenKind::Link => {
                rf.as_generic_link_mut().insert_name(s1);
                rf.as_generic_link_mut().insert_location(location);
                rf.as_generic_link_mut().insert_title(title);
            }
            TokenKind::RefLink => {
                rf.as_generic_link_mut().insert_name(s1);
                rf.as_generic_link_mut().insert_reflink_tag(s2);
            }
            TokenKind::RefLinkDef => {
                rf.as_generic_link_mut().insert_reflink_tag(s1);
                rf.as_generic_link_mut().insert_location(location);
                rf.as_generic_link_mut().insert_title(title);
            }
            TokenKind::QuickLink => {
                rf.as_generic_link_mut().insert_name(s1);
                rf.as_generic_link_mut().insert_location(location);
            }
            TokenKind::Text => {}
            _ => unreachable!(),
        }
        t
    }

    fn tidy(buff: &mut Vec<Token>) {
        Self::tidy_continuous_mark(TokenKind::Star, buff);
        Self::tidy_continuous_mark(TokenKind::UnderLine, buff);

        let mut stack: stack::Stack<&mut Token> = stack::Stack::new();

        let buff_iter = buff.iter_mut().filter(|t| {
            t.kind() == TokenKind::Star
                || t.kind() == TokenKind::UnderLine
                || t.kind() == TokenKind::BackTick
        });

        for t in buff_iter {
            let mut pops = stack.pop_range(|e| e.kind() == t.kind() && e.value() == t.value());
            if !pops.is_empty() {
                // found
                let matched = pops.get_mut(0).unwrap(); // Notice: don't panic
                match t.value() {
                    "*" | "_" => {
                        matched.update_kind(TokenKind::ItalicMark);
                        t.update_kind(TokenKind::ItalicMark);
                    }
                    "**" | "__" => {
                        matched.update_kind(TokenKind::BoldMark);
                        t.update_kind(TokenKind::BoldMark);
                    }
                    "***" | "___" => {
                        matched.update_kind(TokenKind::ItalicBoldMark);
                        t.update_kind(TokenKind::ItalicBoldMark);
                    }
                    "`" | "``" | "```" => {
                        matched.update_kind(TokenKind::CodeMark);
                        t.update_kind(TokenKind::CodeMark);
                    }
                    _ => unreachable!(),
                }
                pops.iter_mut()
                    .skip(1)
                    .for_each(|e| e.update_kind(TokenKind::Text));
            } else {
                // not found in stack
                if t.len() < 4 {
                    stack.push(t);
                } else {
                    t.update_kind(TokenKind::Text)
                }
            }
        }

        // At last, we need to update the elment left on the stack
        stack
            .all_mut()
            .iter_mut()
            .for_each(|e| e.update_kind(TokenKind::Text));
    }

    fn tidy_continuous_mark(kind: TokenKind, buff: &mut Vec<Token>) {
        let mut splits_at: VecDeque<(usize, usize)> = VecDeque::new();

        let mut pre: usize = 0;

        for (ix, t) in buff.iter().enumerate().filter(|(_, t)| t.kind() == kind) {
            let n = if t.len() >= pre && pre > 0 {
                t.len() - pre
            } else {
                pre = t.len();
                continue;
            };
            match n.cmp(&0) {
                // need to split current token
                Ordering::Greater => {
                    let l = splits_at.len();
                    splits_at.push_back((ix + l, pre));

                    pre = n;
                }
                // matched with previous token
                Ordering::Equal => {
                    pre = 0;
                }
                Ordering::Less => unreachable!(),
            }
        }
        splits_at.into_iter().for_each(|(ix, l)| {
            let off = buff[ix].split_off(l);
            buff.insert(ix + 1, off);
        });
    }

    fn goto(&mut self, state: State) {
        self.state = state;
    }

    fn slice_rest(&self, begin: usize) -> &str {
        utf8_slice::from(self.line_text, begin)
    }

    fn slice_str(&self, begin: usize, end: usize) -> &str {
        utf8_slice::slice(self.line_text, begin, end)
    }

    fn slice(&self, begin: usize, end: usize) -> String {
        self.slice_str(begin, end).to_string()
    }

    // find 'line break', double spaces or <br> at the end of the line
    fn has_br(s: &str) -> bool {
        if s.ends_with("  \n") {
            true
        } else {
            s.trim_end().ends_with("<br>")
        }
    }

    fn is_dividing(s: &str) -> bool {
        let counts = s.chars().filter(|c| !c.is_whitespace()).counts();

        counts.len() == 1
            && counts
                .get(&'*')
                .or_else(|| counts.get(&'-'))
                .or_else(|| counts.get(&'_'))
                .copied()
                .unwrap_or(0)
                >= 3
    }

    fn is_quoted_string(s: &str) -> bool {
        D_QUOTED_STRING_RE.is_match(s) || S_QUOTED_STRING_RE.is_match(s)
    }

    fn is_url(s: &str) -> bool {
        Url::try_from(s).is_ok()
    }

    fn is_email(s: &str) -> bool {
        EmailAddress::is_valid(s)
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum TokenKind {
    TitleMark,      // #, ##, ###, ####
    UnorderedMark,  // *
    OrderedMark,    // 1.
    DividingMark,   // ---, ***, ___
    QuoteMark,      // >
    BoldMark,       // ** **
    ItalicMark,     // * *
    ItalicBoldMark, // *** ***
    CodeBlockMark,  // ```
    CodeMark,       // `
    BlankLine,      // \n
    LineBreak,      // <br>, double whitespace
    Image,          // ![name](location "title")
    Link,           // [name](location "title")
    QuickLink,      // <url or email>
    RefLink,        // [name][tag]
    RefLinkDef,     // [tag]: link "title"
    Text,           //
    Star,           // *
    UnderLine,      // _
    BackTick,       // `
    WhiteSpace,     //
}

// Token is a part of the line, the parser will parse the line into some tokens.
#[derive(PartialEq, Debug)]
pub(crate) struct Token {
    value: String,
    kind: TokenKind,
    pub(crate) details: Option<HashMap<String, String>>,
}

impl Token {
    pub(crate) fn new(value: String, kind: TokenKind) -> Self {
        Token {
            value,
            kind,
            details: None,
        }
    }

    // Get value of the token
    pub(crate) fn value(&self) -> &str {
        &self.value
    }

    // Get the value length of the token
    pub(crate) fn len(&self) -> usize {
        self.value.len()
    }

    // Get kind of the token
    pub(crate) fn kind(&self) -> TokenKind {
        self.kind
    }

    // Set a new kind of the token
    pub(crate) fn update_kind(&mut self, kind: TokenKind) {
        self.kind = kind
    }

    // convert the token to generic link token
    pub(crate) fn as_generic_link(&self) -> GenericLinkToken {
        if self.kind() != TokenKind::Link
            && self.kind() != TokenKind::Image
            && self.kind() != TokenKind::RefLink
            && self.kind() != TokenKind::RefLinkDef
            && self.kind() != TokenKind::QuickLink
        {
            panic!("token is not a generic link");
        }
        GenericLinkToken(self)
    }

    fn as_generic_link_mut(&mut self) -> GenericLinkTokenAsMut {
        if self.kind() != TokenKind::Link
            && self.kind() != TokenKind::Image
            && self.kind() != TokenKind::RefLink
            && self.kind() != TokenKind::RefLinkDef
            && self.kind() != TokenKind::QuickLink
        {
            panic!("token is not a generic link");
        }
        GenericLinkTokenAsMut(self)
    }

    fn split_off(&mut self, at: usize) -> Token {
        let off = self.value.split_off(at);
        Token::new(off, self.kind())
    }

    fn insert(&mut self, k: &str, v: &str) {
        self.details
            .get_or_insert(HashMap::new())
            .insert(k.to_string(), v.to_string());
    }
}

// Link Token
#[derive(PartialEq, Debug)]
pub(crate) struct GenericLinkToken<'generic_link_token>(&'generic_link_token Token);

impl<'generic_link_token> GenericLinkToken<'generic_link_token> {
    // Get name of the link
    pub(crate) fn name(&self) -> Option<&str> {
        self.0
            .details
            .as_ref()
            .and_then(|x| x.get("name").map(|x| &**x))
    }

    // Get location of the link
    pub(crate) fn location(&self) -> Option<&str> {
        self.0
            .details
            .as_ref()
            .and_then(|x| x.get("location").map(|x| &**x))
    }

    // Get title of the link
    pub(crate) fn title(&self) -> Option<&str> {
        self.0
            .details
            .as_ref()
            .and_then(|x| x.get("title").map(|x| &**x))
    }
}

#[derive(PartialEq, Debug)]
pub(crate) struct GenericLinkTokenAsMut<'generic_link_token_as_mut>(
    &'generic_link_token_as_mut mut Token,
);

impl<'generic_link_token_as_mut> GenericLinkTokenAsMut<'generic_link_token_as_mut> {
    fn insert_name(&mut self, v: &str) {
        if !v.is_empty() {
            self.0.insert("name", v)
        }
    }

    fn insert_location(&mut self, v: &str) {
        if !v.is_empty() {
            self.0.insert("location", v)
        }
    }

    fn insert_title(&mut self, v: &str) {
        if !v.is_empty() {
            self.0.insert(
                "title",
                v.trim_end_matches(['"', '\''])
                    .trim_start_matches(['"', '\'']),
            )
        }
    }

    fn insert_reflink_tag(&mut self, v: &str) {
        if !v.is_empty() {
            self.0.insert("ptr", v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec_cases(cases: Vec<(&str, Vec<(&str, TokenKind)>)>) {
        for c in cases.iter() {
            let s = if c.0.ends_with('\n') {
                c.0.to_string()
            } else {
                let mut s1 = c.0.to_string();
                s1.push('\n');
                s1
            };

            assert_eq!(
                Lexer::new(s.as_str()).split(),
                c.1.iter()
                    .map(|(v, k)| { Token::new(v.to_string(), *k) })
                    .collect::<Vec<Token>>()
            );
        }
    }

    fn exec_generic_link_cases(cases: Vec<(&str, Vec<(&str, TokenKind, &str, &str, &str)>)>) {
        for c in cases.iter() {
            let s = if c.0.ends_with('\n') {
                c.0.to_string()
            } else {
                let mut s1 = c.0.to_string();
                s1.push('\n');
                s1
            };

            assert_eq!(
                Lexer::new(s.as_str()).split(),
                c.1.iter()
                    .map(|(v, k, s1, s2, s3)| {
                        let mut t = Token::new(v.to_string(), *k);
                        match k {
                            TokenKind::Link | TokenKind::Image | TokenKind::QuickLink => {
                                let mut tm = t.as_generic_link_mut();
                                tm.insert_name(s1);
                                tm.insert_location(s2);
                                tm.insert_title(s3);
                            }
                            TokenKind::RefLink => {
                                let mut tm = t.as_generic_link_mut();
                                tm.insert_name(s1);
                                tm.insert_reflink_tag(s2)
                            }
                            TokenKind::RefLinkDef => {
                                let mut tm = t.as_generic_link_mut();
                                tm.insert_reflink_tag(s1);
                                tm.insert_location(s2);
                                tm.insert_title(s3);
                            }
                            _ => (),
                        }
                        t
                    })
                    .collect::<Vec<Token>>()
            );
        }
    }

    #[test]
    fn test_normal_text() {
        let cases = vec![
            ("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。",
                vec![("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。", TokenKind::Text)],
            ),
            ("--- x", vec![("--- x", TokenKind::Text)]),
            (
                "___ 这不是一个分界线",
                vec![
                    ("___", TokenKind::Text),
                    (" 这不是一个分界线", TokenKind::Text),
                ],
            ),
            ("#这不是标题", vec![("#这不是标题", TokenKind::Text)]),
            ("##这也不是标题", vec![("##这也不是标题", TokenKind::Text)]),
            (">这不是引用", vec![(">这不是引用", TokenKind::Text)]),
            ("1.这也不是列表", vec![("1.这也不是列表", TokenKind::Text)]),
            (
                "***xxxx",
                vec![("***", TokenKind::Text), ("xxxx", TokenKind::Text)],
            ),
        ];

        exec_cases(cases);
    }

    #[test]
    fn test_bold_italic() {
        let cases = vec![
            (
                "**粗体**_斜体_***斜体+粗体***",
                vec![
                    ("**", TokenKind::BoldMark),
                    ("粗体", TokenKind::Text),
                    ("**", TokenKind::BoldMark),
                    ("_", TokenKind::ItalicMark),
                    ("斜体", TokenKind::Text),
                    ("_", TokenKind::ItalicMark),
                    ("***", TokenKind::ItalicBoldMark),
                    ("斜体+粗体", TokenKind::Text),
                    ("***", TokenKind::ItalicBoldMark),
                ],
            ),
            (
                "**1** ****2***",
                vec![
                    ("**", TokenKind::BoldMark),
                    ("1", TokenKind::Text),
                    ("**", TokenKind::BoldMark),
                    (" ", TokenKind::Text),
                    ("****", TokenKind::Text),
                    ("2", TokenKind::Text),
                    ("***", TokenKind::Text),
                ],
            ),
            (
                "**__2__**",
                vec![
                    ("**", TokenKind::BoldMark),
                    ("__", TokenKind::BoldMark),
                    ("2", TokenKind::Text),
                    ("__", TokenKind::BoldMark),
                    ("**", TokenKind::BoldMark),
                ],
            ),
            (
                "**1**",
                vec![
                    ("**", TokenKind::BoldMark),
                    ("1", TokenKind::Text),
                    ("**", TokenKind::BoldMark),
                ],
            ),
            (
                "*1*",
                vec![
                    ("*", TokenKind::ItalicMark),
                    ("1", TokenKind::Text),
                    ("*", TokenKind::ItalicMark),
                ],
            ),
            (
                "*** 1 ***",
                vec![
                    ("***", TokenKind::ItalicBoldMark),
                    (" 1 ", TokenKind::Text),
                    ("***", TokenKind::ItalicBoldMark),
                ],
            ),
            (
                "__1__",
                vec![
                    ("__", TokenKind::BoldMark),
                    ("1", TokenKind::Text),
                    ("__", TokenKind::BoldMark),
                ],
            ),
            (
                "_1_",
                vec![
                    ("_", TokenKind::ItalicMark),
                    ("1", TokenKind::Text),
                    ("_", TokenKind::ItalicMark),
                ],
            ),
            (
                "___ 1 ___",
                vec![
                    ("___", TokenKind::ItalicBoldMark),
                    (" 1 ", TokenKind::Text),
                    ("___", TokenKind::ItalicBoldMark),
                ],
            ),
        ];

        exec_cases(cases);
    }

    #[test]
    fn test_line_break() {
        let cases = vec![
            ("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。  ",  // have two spaces at the end of the line.
                vec![("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。", TokenKind::Text),
                    ("<br>", TokenKind::LineBreak)],
            ),
            ("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。       ", // have two tab spaces at the end of the line.
                vec![("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。", TokenKind::Text),
                    ("<br>", TokenKind::LineBreak)],
            ),
            ("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。    <br>  ", // have two spaces at the end of the line.
                vec![("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。    ", TokenKind::Text),
                    ("<br>", TokenKind::LineBreak)],
            ),
            ("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。<br>",
                vec![("这是我的一个学习 rust 编程语言的项目，我将尝试去开发一个强大的 markdown 编辑器。", TokenKind::Text),
                    ("<br>", TokenKind::LineBreak)],
            ),
        ];

        exec_cases(cases);
    }

    #[test]
    fn test_blank_line() {
        let cases = vec![
            ("\n", vec![("", TokenKind::BlankLine)]),
            (" \n", vec![(" ", TokenKind::BlankLine)]),
            ("     \n", vec![("     ", TokenKind::BlankLine)]),
            ("         ", vec![("         ", TokenKind::BlankLine)]),
            (
                "                                            ",
                vec![(
                    "                                            ",
                    TokenKind::BlankLine,
                )],
            ),
            ("  ", vec![("  ", TokenKind::BlankLine)]),
        ];

        exec_cases(cases);
    }

    #[test]
    fn test_dividing() {
        let cases = vec![
            ("---", vec![("---", TokenKind::DividingMark)]),
            ("***", vec![("***", TokenKind::DividingMark)]),
            ("___", vec![("___", TokenKind::DividingMark)]),
            ("- -----", vec![("- -----", TokenKind::DividingMark)]),
            ("* * *", vec![("* * *", TokenKind::DividingMark)]),
            (
                "__ ________         ",
                vec![("__ ________         ", TokenKind::DividingMark)],
            ),
            (
                "----------------------------------------   ",
                vec![(
                    "----------------------------------------   ",
                    TokenKind::DividingMark,
                )],
            ),
        ];

        exec_cases(cases);
    }

    #[test]
    fn test_code_in_line() {
        let cases = vec![
            (
                "`rust`",
                vec![
                    ("`", TokenKind::CodeMark),
                    ("rust", TokenKind::Text),
                    ("`", TokenKind::CodeMark),
                ],
            ),
            (
                "``rust``",
                vec![
                    ("``", TokenKind::CodeMark),
                    ("rust", TokenKind::Text),
                    ("``", TokenKind::CodeMark),
                ],
            ),
            (
                "rust```rust```",
                vec![
                    ("rust", TokenKind::Text),
                    ("```", TokenKind::CodeMark),
                    ("rust", TokenKind::Text),
                    ("```", TokenKind::CodeMark),
                ],
            ),
        ];

        exec_cases(cases);
    }

    #[test]
    fn test_escape() {
        let cases = vec![
            (
                "\\`rust`",
                vec![("`rust", TokenKind::Text), ("`", TokenKind::Text)],
            ),
            (
                "\\`rust\\`",
                vec![("`rust", TokenKind::Text), ("`", TokenKind::Text)],
            ),
            (
                "\\***rust***",
                vec![
                    ("*", TokenKind::Text),
                    ("**", TokenKind::BoldMark),
                    ("rust", TokenKind::Text),
                    ("**", TokenKind::BoldMark),
                    ("*", TokenKind::Text),
                ],
            ),
            (
                "\\***rust**\\*",
                vec![
                    ("*", TokenKind::Text),
                    ("**", TokenKind::BoldMark),
                    ("rust", TokenKind::Text),
                    ("**", TokenKind::BoldMark),
                    ("*", TokenKind::Text),
                ],
            ),
        ];

        exec_cases(cases);
    }

    #[test]
    fn test_image() {
        let cases = vec![
            (
                r#"![这是图片](/assets/img/philly-magic-garden.jpg "Magic Gardens")"#,
                vec![(
                    r#"![这是图片](/assets/img/philly-magic-garden.jpg "Magic Gardens")"#,
                    TokenKind::Image,
                    "这是图片",
                    "/assets/img/philly-magic-garden.jpg",
                    "Magic Gardens",
                )],
            ),
            (
                r#"![](/assets/img/philly-magic-garden.jpg "Magic Gardens")"#,
                vec![(
                    r#"![](/assets/img/philly-magic-garden.jpg "Magic Gardens")"#,
                    TokenKind::Image,
                    "",
                    "/assets/img/philly-magic-garden.jpg",
                    "Magic Gardens",
                )],
            ),
            (
                r#"![](/assets/img/philly-magic-garden.jpg 'Magic Gardens')"#,
                vec![(
                    r#"![](/assets/img/philly-magic-garden.jpg 'Magic Gardens')"#,
                    TokenKind::Image,
                    "",
                    "/assets/img/philly-magic-garden.jpg",
                    "Magic Gardens",
                )],
            ),
            (
                "![](/assets/img/philly-magic-garden.jpg Magic Gardens)",
                vec![(
                    "![](/assets/img/philly-magic-garden.jpg Magic Gardens)",
                    TokenKind::Text,
                    "",
                    "",
                    "",
                )],
            ),
            (
                r#"![](/assets/img/philly-magic-garden.jpg "Magic" Gardens)"#,
                vec![(
                    r#"![](/assets/img/philly-magic-garden.jpg "Magic" Gardens)"#,
                    TokenKind::Text,
                    "",
                    "",
                    "",
                )],
            ),
            (
                r#"![](/assets/img/philly-magic-garden.jpg "Magic Gardens)"#,
                vec![(
                    r#"![](/assets/img/philly-magic-garden.jpg "Magic Gardens)"#,
                    TokenKind::Text,
                    "",
                    "",
                    "",
                )],
            ),
            ("![]()", vec![("![]()", TokenKind::Image, "", "", "")]),
            (
                "![[[[[[]()",
                vec![("![[[[[[]()", TokenKind::Image, "[[[[[", "", "")],
            ),
            ("![[]]()", vec![("![[]]()", TokenKind::Image, "[]", "", "")]),
            ("![!]()", vec![("![!]()", TokenKind::Image, "!", "", "")]),
            (
                "![![]]()",
                vec![("![![]]()", TokenKind::Image, "![]", "", "")],
            ),
        ];

        exec_generic_link_cases(cases);
    }

    #[test]
    fn test_link() {
        let cases = vec![
            (
                "[这是链接](/assets/img/philly-magic-garden.jpg \"Magic Gardens\")",
                vec![(
                    "[这是链接](/assets/img/philly-magic-garden.jpg \"Magic Gardens\")",
                    TokenKind::Link,
                    "这是链接",
                    "/assets/img/philly-magic-garden.jpg",
                    "Magic Gardens",
                )],
            ),
            (
                "[](/assets/img/philly-magic-garden.jpg \"Magic Gardens\")",
                vec![(
                    "[](/assets/img/philly-magic-garden.jpg \"Magic Gardens\")",
                    TokenKind::Link,
                    "",
                    "/assets/img/philly-magic-garden.jpg",
                    "Magic Gardens",
                )],
            ),
            ("[]()", vec![("[]()", TokenKind::Link, "", "", "")]),
            ("[]]()", vec![("[]]()", TokenKind::Link, "]", "", "")]),
            ("[]]]]()", vec![("[]]]]()", TokenKind::Link, "]]]", "", "")]),
            ("[!]]()", vec![("[!]]()", TokenKind::Link, "!]", "", "")]),
        ];

        exec_generic_link_cases(cases);
    }

    #[test]
    fn test_title() {
        let cases = vec![
            (
                "# header1",
                vec![("#", TokenKind::TitleMark), ("header1", TokenKind::Text)],
            ),
            (
                "## header2",
                vec![("##", TokenKind::TitleMark), ("header2", TokenKind::Text)],
            ),
            (
                "### header3 header3",
                vec![
                    ("###", TokenKind::TitleMark),
                    ("header3 header3", TokenKind::Text),
                ],
            ),
            (
                "####  header4",
                vec![
                    ("####", TokenKind::TitleMark),
                    (" header4", TokenKind::Text),
                ],
            ),
            ("# ", vec![("#", TokenKind::TitleMark)]),
            ("#  ", vec![("#", TokenKind::TitleMark)]),
        ];
        exec_cases(cases);
    }

    #[test]
    fn test_quote() {
        let cases = vec![(
            "> Rust, A language empowering everyone to build reliable and efficient software.",
            vec![(">", TokenKind::QuoteMark), (
                "Rust, A language empowering everyone to build reliable and efficient software.",
                TokenKind::Text,
            )],
        )];
        exec_cases(cases);
    }

    #[test]
    fn test_unordered_list() {
        let cases = vec![(
            "* rust",
            vec![("*", TokenKind::UnorderedMark), ("rust", TokenKind::Text)],
        )];
        exec_cases(cases);
    }

    #[test]
    fn test_ordered_list() {
        let cases = vec![
            (
                "1. rust",
                vec![("1.", TokenKind::OrderedMark), ("rust", TokenKind::Text)],
            ),
            (
                "2. rust",
                vec![("2.", TokenKind::OrderedMark), ("rust", TokenKind::Text)],
            ),
            (
                "3. rust",
                vec![("3.", TokenKind::OrderedMark), ("rust", TokenKind::Text)],
            ),
            (
                "10. rust",
                vec![("10.", TokenKind::OrderedMark), ("rust", TokenKind::Text)],
            ),
            (
                "20. rust",
                vec![("20.", TokenKind::OrderedMark), ("rust", TokenKind::Text)],
            ),
            (
                "100. rust",
                vec![("100.", TokenKind::OrderedMark), ("rust", TokenKind::Text)],
            ),
            (
                "999. rust",
                vec![("999.", TokenKind::OrderedMark), ("rust", TokenKind::Text)],
            ),
        ];
        exec_cases(cases);
    }

    #[test]
    fn test_code_block_mark() {
        let cases = vec![
            ("```", vec![("```", TokenKind::CodeBlockMark)]),
            (
                "```rust",
                vec![("```", TokenKind::CodeBlockMark), ("rust", TokenKind::Text)],
            ),
        ];
        exec_cases(cases);
    }

    #[test]
    fn test_auto_link() {
        let cases = vec![
            ("<>", vec![("<>", TokenKind::Text, "", "", "")]),
            (
                "<https://example.com",
                vec![("<https://example.com", TokenKind::Text, "", "", "")],
            ),
            (
                "<https://example.com>",
                vec![(
                    "<https://example.com>",
                    TokenKind::QuickLink,
                    "https://example.com",
                    "https://example.com",
                    "",
                )],
            ),
            (
                "<  https://example.com >",
                vec![(
                    "<  https://example.com >",
                    TokenKind::QuickLink,
                    "https://example.com",
                    "https://example.com",
                    "",
                )],
            ),
            (
                "auto link <  https://example.com >!",
                vec![
                    ("auto link ", TokenKind::Text, "", "", ""),
                    (
                        "<  https://example.com >",
                        TokenKind::QuickLink,
                        "https://example.com",
                        "https://example.com",
                        "",
                    ),
                    ("!", TokenKind::Text, "", "", ""),
                ],
            ),
            (
                "<user@example.com>",
                vec![(
                    "<user@example.com>",
                    TokenKind::QuickLink,
                    "user@example.com",
                    "user@example.com",
                    "",
                )],
            ),
        ];
        exec_generic_link_cases(cases);
    }

    #[test]
    fn test_reflink() {
        let cases = vec![
            (
                "[Example][link]",
                vec![("[Example][link]", TokenKind::RefLink, "Example", "link", "")],
            ),
            (
                "link: [Example][link].",
                vec![
                    ("link: ", TokenKind::Text, "", "", ""),
                    ("[Example][link]", TokenKind::RefLink, "Example", "link", ""),
                    (".", TokenKind::Text, "", "", ""),
                ],
            ),
        ];
        exec_generic_link_cases(cases);
    }

    #[test]
    fn test_reflink_def() {
        let cases = vec![
            (
                "[link]: https://example.com",
                vec![(
                    "[link]: https://example.com",
                    TokenKind::RefLinkDef,
                    "link",
                    "https://example.com",
                    "",
                )],
            ),
            (
                "[link]: https://example.com \"example\"",
                vec![(
                    "[link]: https://example.com \"example\"",
                    TokenKind::RefLinkDef,
                    "link",
                    "https://example.com",
                    "example",
                )],
            ),
        ];
        exec_generic_link_cases(cases);
    }
}
