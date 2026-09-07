use crate::{ErrType, ProgramError, ProgramErrorData, lexing::tokens::{Span, TokenKind, TokenSpan}};

pub mod tokens;


pub struct Lexer<'a> {
    err_data: &'a mut ProgramErrorData,
    source: &'a str,
    tokens: Vec<TokenSpan>,
    line_start_lookup: Vec<usize>,
    byte_pos: usize,
    curr_line: usize,
    curr_token_start: usize
}

impl Lexer<'_> {
    pub fn start(err_data: &mut ProgramErrorData, source: &str) -> (Vec<TokenSpan>, Vec<usize>) {
        let mut lexer = Lexer {
            err_data, source,
            tokens: Vec::new(),
            line_start_lookup: vec![0],
            byte_pos: 0,
            curr_line: 1,
            curr_token_start: 0
        };
        lexer.tokenize(None);

        (lexer.tokens, lexer.line_start_lookup)
    }

    #[track_caller]
    fn error(&mut self, err_type: ErrType) {
        self.err_data.errors.push(ProgramError {
            span: Span {
                line: self.curr_line,
                byte_offset: self.byte_pos,
                length: self.byte_pos - self.curr_token_start + 1
            },
            err_type,
            compiler_location: std::panic::Location::caller()
        });
    }


    fn tokenize(&mut self, mut brace_level: Option<usize>) {
        loop {
            self.skip_whitespaces();
            self.curr_token_start = self.byte_pos;

            let remaining = &self.source[self.byte_pos..];
            let Some(first_char) = remaining.chars().next() else {
                // remaining was empty => EndOfFile
                return
            };

            // comments
            if remaining.starts_with("//") { self.skip_comment("\n"); continue }
            if remaining.starts_with("/*") { self.skip_comment("*/"); continue }

            // check if the start of remaining matches any Punctuation token
            if let Some(&(s, kind)) = TokenKind::PUNCTUATION.iter().find(|(p, _)| remaining.starts_with(p)) {
                if let Some(bl) = &mut brace_level {
                    if let TokenKind::LeftBrace = kind {
                        *bl += 1;
                    }
                    if let TokenKind::RightBrace = kind {
                        if *bl == 0 { return }
                        *bl -= 1;
                    }
                }

                self.byte_pos += s.len();
                self.add_token(kind);
                continue;
            }

            match first_char {
                // identifiers / keywords
                c if c.is_alphabetic() || c == '_' => {
                    let ident = self.eat_identifier();

                    // is the identifier a keyword?
                    if let Some(&(_, kind)) = TokenKind::KEYWORDS.iter().find(|(kw, _)| *kw == ident) {
                        self.add_token(kind);
                    } else {
                        // the actual name will be gotten from the source code with the TokenSpan
                        self.add_token(TokenKind::Identifier);
                    }
                }
                // quotes
                '"' => self.eat_string(b'"'),
                '\'' => self.eat_string(b'\''),

                // numbers
                '0'..='9' => self.eat_number(),

                c => {
                    self.error(ErrType::LexerUnexpectedCharacter { c });
                    self.byte_pos += c.len_utf8();
                }
            }
        }
    }


    fn skip_whitespaces(&mut self) {
        loop {
            let remaining = &self.source[self.byte_pos..];
            if remaining.starts_with([' ', '\r', '\t']) {
                self.byte_pos += 1;
            }
            else if remaining.starts_with('\n') {
                self.byte_pos += 1;
                self.curr_line += 1;
                self.line_start_lookup.push(self.byte_pos);
            }
            else if remaining.starts_with("//") { self.skip_comment("\n"); }
            else if remaining.starts_with("/*") { self.skip_comment("*/"); }
            // nothing to skip
            else { break }
        }
    }

    fn skip_comment(&mut self, end_str: &str) {
        while self.byte_pos < self.source.len() {
            let remaining = &self.source[self.byte_pos..];
            if remaining.starts_with('\n') {
                self.curr_line += 1;
                self.line_start_lookup.push(self.byte_pos + 1);
            }
            if remaining.starts_with(end_str) {
                self.byte_pos += end_str.len();
                break;
            }
            self.byte_pos += 1;
        }
        self.curr_token_start = self.byte_pos;
    }

    fn add_token(&mut self, token_type: TokenKind) {
        let new_token = TokenSpan {
            token: token_type,
            span: Span {
                line: self.curr_line,
                byte_offset: self.curr_token_start,
                length: self.byte_pos - self.curr_token_start,
            }
        };
        self.tokens.push(new_token);
    }


    fn eat_identifier(&mut self) -> &str {
        let start_byte = self.byte_pos;

        for c in self.source[self.byte_pos..].chars() {
            if c.is_alphanumeric() || c == '_' {
                self.byte_pos += c.len_utf8();
            } else {
                break;
            }
        }
        &self.source[start_byte..self.byte_pos]
    }


    fn eat_string(&mut self, quote: u8) {
        self.byte_pos += 1;
        self.add_token(TokenKind::StringStart);
        self.curr_token_start = self.byte_pos;
        let mut is_backslashed = false;

        while let Some(&b) = self.source.as_bytes().get(self.byte_pos) {
            if is_backslashed {
                is_backslashed = false;
                self.byte_pos += 1;
                continue;
            }

            match b {
                _ if b == quote => {
                    // quote ends!

                    // don't add empty strings
                    if self.curr_token_start != self.byte_pos {
                        self.add_token(TokenKind::StringFrag);
                    }

                    self.byte_pos += 1;
                    self.add_token(TokenKind::StringEnd);
                    return;
                }
                b'{' => {
                    // don't add empty strings
                    if self.curr_token_start != self.byte_pos {
                        self.add_token(TokenKind::StringFrag);
                    }

                    // start template string stuff
                    self.byte_pos += 1;  // eats '{'
                    self.add_token(TokenKind::LeftBrace);
                    self.tokenize(Some(0)); // recursion for template strings
                    match self.source.as_bytes().get(self.byte_pos) {
                        Some(b'}') => {
                            self.byte_pos += 1;  // eats '}'
                            self.add_token(TokenKind::RightBrace);
                            self.curr_token_start = self.byte_pos;  // set the token_start right after '}'
                        }
                        _ => return self.error(ErrType::LexerMissingClosingStringBrace)
                    }
                }
                b'\\' => {
                    is_backslashed = true;
                    self.byte_pos += 1;
                }

                _ => self.byte_pos += 1,
            }
        }

        self.error(ErrType::LexerUnterminatedString);
    }


    fn eat_number(&mut self) {
        let mut has_dot = false;
        let bytes = self.source.as_bytes();

        while let Some(&b) = bytes.get(self.byte_pos) {
            if let b'_' | b'0'..=b'9' = b {
                self.byte_pos += 1;
            }
            else if b == b'.' && !has_dot
                // peek ahead to check if there is a digit after the '.'
                && let Some(b'0'..=b'9') = bytes.get(self.byte_pos + 1) {
                    has_dot = true;
                    self.byte_pos += 2;
                }
            else {
                break
            }
        }
        // let text_slice = &self.source[start..self.byte_pos];

        self.add_token(if has_dot { TokenKind::NumFloat } else { TokenKind::NumInt });
    }
}


#[must_use]
pub fn lex_string_from(source_frag: &str) -> String {
    let mut s = String::with_capacity(source_frag.len());
    let mut is_backslashed = false;
    let mut skip_spaces = false;

    for c in source_frag.chars() {
        if is_backslashed {
            match c {
                'n' => s.push('\n'),
                't' => s.push('\t'),
                'r' => s.push('\r'),
                '0' => s.push('\0'),
                '\n' => {
                    s.push('\n');
                    skip_spaces = true;
                }
                _ => s.push(c),
            }
            is_backslashed = false;
            continue
        }
        if skip_spaces {
            if let ' ' | '\r' | '\t' = c { continue }
            skip_spaces = false;
        }
        s.push(c);
    }
    s.shrink_to_fit();
    s
}


#[must_use]
pub fn lex_num_int_from(source_frag: &str) -> i64 {
    source_frag
        .chars()
        .filter(|&c| c != '_')
        .map(|x| x.to_digit(10).unwrap_or_else(|| unreachable!("Other characters should not exist in ints... {source_frag:?} {x}")))
        .fold(0, |acc, digit| acc * 10 + i64::from(digit))
}


#[must_use]
pub fn lex_num_float_from(source_frag: &str) -> f64 {
    source_frag
        .parse()
        .unwrap_or_else(|x| unreachable!("{source_frag:?} {x}"))
}