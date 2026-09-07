use derive_more::Display;


#[derive(Debug, Display, Clone)]
#[display("({token} - {span})")]
pub struct TokenSpan {
    pub token: TokenKind,
    // where its located in the file, for errors
    pub span: Span,
}
impl TokenSpan {
    pub const END_TOKEN: Self = Self { token: TokenKind::EndOfFile, span: Span::invalid() };
}


#[derive(Debug, Display, Clone, Copy, PartialEq)]
#[display("{}",
    Self::PUNCTUATION.iter()
        .chain(Self::KEYWORDS)
        .find_map(|(s, kind)| (kind == self).then_some(s))
        .unwrap_or_else(|| panic!("i forgot to handle that variant... {self:?}"))
)]
pub enum TokenKind {
    // Basic
    LeftParen, RightParen,
    LeftBracket, RightBracket,
    LeftBrace, RightBrace,
    Comma, Semicolon,
    Colon, ColonColon,

    // Operators
    Assign { extra_op: Option<AssignOp> },
    Op(AssignOp),

    // Bitwise
    // BitNot,
    // BitAnd,
    // BitOr,
    // BitXor,
    // LeftShift,
    // RightShift,

    // Logical
    EqualEqual,
    Exclamation, NotEqual,
    Less, LessEqual,
    Greater, GreaterEqual,

    // Advanced
    Pipe,
    MinusArrow, EqualArrow,
    Caret,
    Quest,
    Hashtag,
    Dot,
    DotDot, DotDotEqual,
    DotDotDot,
    Ampersand,
    #[display("<eof>")]
    EndOfFile,

    #[display("<ident>")]
    Identifier,  // these names dont need to be stored here (its already stored in the source code)
    #[display("<int>")]
    NumInt,
    #[display("<float>")]
    NumFloat,
    #[display("<stringStart>")]
    StringStart,
    #[display("<stringFrag>")]
    // stringfrags needs to be lexed later in the parser.
    // removing (String) from here makes this enum TINY though!!
    StringFrag,
    #[display("<stringEnd>")]
    StringEnd,
    Bool(bool),

    // Keywords
    And, Or,
    If, Else, Ensure,
    For, In, While, Loop,
    Break, Continue,
    Fn, Return,
    ImplSelf,
    Match, Is,
    Let, Const, Type,
    Mut, Ref, Own,
    Enum, Impl,
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AssignOp {
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    QuestQuest,
}

impl TokenKind {
    pub const PUNCTUATION: &[(&'static str, Self)] = &[
        // Longer tokens need to go first otherwise it picks '-' over '->'

        // Basic
        ("(", Self::LeftParen), (")", Self::RightParen),
        ("[", Self::LeftBracket), ("]", Self::RightBracket),
        ("{", Self::LeftBrace), ("}", Self::RightBrace),
        (",", Self::Comma), (";", Self::Semicolon),
        ("::", Self::ColonColon), (":", Self::Colon),

        // Advanced
        ("|", Self::Pipe),
        ("->", Self::MinusArrow), ("=>", Self::EqualArrow),
        ("^", Self::Caret),
        ("?", Self::Quest),
        ("#", Self::Hashtag),
        ("...", Self::DotDotDot),
        ("..=", Self::DotDotEqual), ("..", Self::DotDot),
        (".", Self::Dot),
        ("&", Self::Ampersand),

        // Operators
        ("+=", Self::Assign { extra_op: Some(AssignOp::Plus) }), ("+", Self::Op(AssignOp::Plus)),
        ("-=", Self::Assign { extra_op: Some(AssignOp::Minus) }), ("-", Self::Op(AssignOp::Minus)),
        ("*=", Self::Assign { extra_op: Some(AssignOp::Star) }), ("*", Self::Op(AssignOp::Star)),
        ("/=", Self::Assign { extra_op: Some(AssignOp::Slash) }), ("/", Self::Op(AssignOp::Slash)),
        ("%=", Self::Assign { extra_op: Some(AssignOp::Percent) }), ("%", Self::Op(AssignOp::Percent)),
        ("??=", Self::Assign { extra_op: Some(AssignOp::QuestQuest) }), ("??", Self::Op(AssignOp::QuestQuest)),

        // Logical
        ("==", Self::EqualEqual), ("=", Self::Assign { extra_op: None }),
        ("!=", Self::NotEqual), ("!", Self::Exclamation),
        ("<=", Self::LessEqual), ("<", Self::Less),
        (">=", Self::GreaterEqual), (">", Self::Greater),
    ];

    pub const KEYWORDS: &[(&'static str, Self)] = &[
        // Keywords
        ("and", Self::And), ("or", Self::Or),
        ("if", Self::If), ("else", Self::Else), ("ensure", Self::Ensure),
        ("for", Self::For), ("in", Self::In), ("while", Self::While), ("loop", Self::Loop),
        ("break", Self::Break), ("continue", Self::Continue),
        ("fn", Self::Fn), ("return", Self::Return),
        ("Self", Self::ImplSelf),
        ("match", Self::Match), ("is", Self::Is),
        ("let", Self::Let), ("const", Self::Const), ("type", Self::Type),
        ("mut", Self::Mut), ("ref", Self::Ref), ("own", Self::Own),
        ("enum", Self::Enum), ("impl", Self::Impl),

        // Literals
        ("true", Self::Bool(true)), ("false", Self::Bool(false)),
    ];
}


#[derive(Debug, Display, Clone, Copy, Default)]
#[display("{line} {byte_offset} {length}")]
pub struct Span {
    pub line: usize,
    pub byte_offset: usize,
    pub length: usize,
}
impl Span {
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        // |----------| (span self)
        // 219029812813 + (12321 * 1259812895)
        //                 |----------------| (span other)
        // merged span:
        // |--------------------------------|
        let start_byte = self.byte_offset.min(other.byte_offset);
        let end_byte = self.byte_offset.saturating_add(self.length)
                        .max(other.byte_offset.saturating_add(other.length));
        Self {
            line: self.line.min(other.line),
            byte_offset: start_byte,
            length: end_byte - start_byte,
        }
    }
    #[must_use]
    pub const fn to_0_width_right(self) -> Self {
        Self {
            line: self.line,
            // saturating add, because this function should work
            // for Invalid Span tokens, e.g. <eof>
            byte_offset: self.byte_offset.saturating_add(self.length),
            length: 0
        }
    }
    #[must_use]
    pub const fn invalid() -> Self {
        Self { line: usize::MAX, byte_offset: usize::MAX, length: usize::MAX }
    }
}