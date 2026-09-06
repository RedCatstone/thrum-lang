use strum_macros::IntoStaticStr;
use crate::lexing::tokens::{AssignOp, Span, TokenKind};

pub type AstIds = u32;

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct ExprId(pub AstIds);
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct PatternId(pub AstIds);


// this is where the entire Abstract Syntax Tree is stored, just one vec.
// ast nodes can reference their children with indices
#[derive(Default, Debug)]
pub struct AstArena {
    pub exprs: Vec<Expr>,
    pub expr_spans: Vec<Span>,

    pub patterns: Vec<Pattern>,
    pub pattern_spans: Vec<Span>,
}

impl AstArena {
    pub fn add_expr(&mut self, span: Span, expr: Expr) -> ExprId {
        debug_assert_eq!(self.exprs.len(), self.expr_spans.len());
        let id = self.exprs.len().try_into().unwrap();
        self.exprs.push(expr);
        self.expr_spans.push(span);
        ExprId(id)
    }
    #[must_use] pub fn get_expr     (&self, id: ExprId) -> &Expr { &self.exprs    [id.0 as usize] }
    #[must_use] pub fn get_expr_span(&self, id: ExprId) -> Span  { self.expr_spans[id.0 as usize] }

    pub fn add_pattern(&mut self, span: Span, pattern: Pattern) -> PatternId {
        debug_assert_eq!(self.patterns.len(), self.pattern_spans.len());
        let id =  self.patterns.len().try_into().unwrap();
        self.patterns.push(pattern);
        self.pattern_spans.push(span);
        PatternId(id)
    }
    #[must_use] pub fn get_pattern     (&self, id: PatternId) -> &Pattern { &self.patterns    [id.0 as usize] }
    #[must_use] pub fn get_pattern_span(&self, id: PatternId) -> Span     { self.pattern_spans[id.0 as usize] }
}


// Everything is an expression.
#[derive(Debug, IntoStaticStr, Clone)]
pub enum Expr {
    Literal { val: AstValue },  // 2, "bla", true
    Prefix { op: TokenKind, right: ExprId },  // !a
    Infix  { op: TokenKind, op_span: Span, left: ExprId, right: ExprId },  // a + b

    Block { exprs: Vec<ExprId>, label: Option<String> },  // { ... }
    IdentifierRef { name: String, mutable: bool },  // x  or  mut x
    Assign { pattern: PatternId, value: ExprId, extra_op: Option<AssignOp>, op_span: Span }, // x = 2  or  let x = 2
    EmptyLet { pattern: PatternId },  // let x
    Const { pattern: PatternId, value: ExprId },  // const x = 5
    CustomType { name: Box<str>, value: ExprId },  // type x = 5
    Move { expr: ExprId },  // x^
    Borrow { expr: ExprId, mutable: bool },  // &u32
    MemberAccess { left: ExprId, member: String },  // arr.len
    TypeMemberAccess { left: ExprId, member: String },  // Option::Some

    TemplateString { elems: Vec<ExprId> },  // "a{b}c" -> [Literal("a"), IdentifierRef("b"), Literal("c")]
    Tuple { elems: Vec<AstTupleElement> },  // (1, 2)
    TupleArr { elem: ExprId, length: ExprId },  // (0; 4)
    Index { left: ExprId, index: ExprId },  // arr[1]

    If { condition: ExprId, then: ExprId, alt: ExprId, never_alt: bool },  // if true { ... } else ... (alt is void if not present)
    Is { value: ExprId, pattern: PatternId },  // queue.pop() is let .Some(x)
    Match { match_value: ExprId, arms: Vec<AstMatchArm> },  // match response { 2 -> "success", _ -> "nope." }

    While { condition: ExprId, body: ExprId, label: String, },  // while true { ... }
    For { pattern: PatternId, iter_expr: ExprId, body: ExprId, label: String },  // for x in 0..5 { ... }
    Loop { body: ExprId, label: String },  // loop { ... }

    FnDefinition { name: Box<str>, closure: AstClosure },  // fn square(x: num) { x**2 }
    Closure { closure: AstClosure, requires_type_annotation: bool },  // |x -> x**2
    Call { callee: ExprId, arguments: Vec<ExprId> },  // x(1, 2)
    TypeInstantiation { typ: ExprId, data: ExprId /* tuple */ },  // Point{ x: 1, y: 2 }
    EnumDefinition { variants: Vec<AstEnumExpression> },  // enum Color { Red, Blue, Green(data) }
    EnumVariant { data: AstEnumExpression },  // .North

    ImplBlock { typ: ExprId, const_exprs: Vec<ExprId> },  // impl Expr { ... }
    ImplSelf,

    Return { expr: ExprId },  // return ...
    Break { label: Option<String>, expr: ExprId },  // break #label ...
    Continue { label: Option<String> },  // continue #label

    Void,  // Semicolons are void expressions.
    ParserError,  // desugar and typechecking will just ignore these nodes
}




#[derive(Debug, Clone)]
pub enum AstValue {
    NumInt(i64),
    NumFloat(f64),
    Str(String),
    Bool(bool),
}





#[derive(Debug, IntoStaticStr, Clone)]
pub enum Pattern {
    Wildcard,  // _
    Not(PatternId),  // !...

    Or(Vec<PatternId>),  // ... | ...
    Tuple(Vec<AstTuplePattern>),  // (...)
    String { before: String, hole_parts: Box<[(PatternId, String)]> },  // "hello {name}!"

    Binding { name: Box<str>, mutable: bool },  // x: num
    TypeDestructor { typ: ExprId, data: PatternId /* tuple */ },  // Point{ x: 1, y: 2 }
    EnumVariant { name: String, attached_tuple: Option<PatternId> },  // .North{ 2 }
    Conditional { pattern: PatternId, cond: ExprId },

    Typed { pattern: PatternId, typ: ExprId },

    CompareExpr(ExprId),  // x is 3
    PlacePointer(ExprId),  // x = ...
}





// random simple structs
#[derive(Debug, Clone)]
pub struct AstClosure {
    pub params: Box<[PatternId]>,
    pub return_type: Option<ExprId>,
    pub body: ExprId,
}

#[derive(Debug, Clone)]
pub struct AstTupleElement {
    pub label: String,
    pub expr: ExprId,
}
#[derive(Debug, Clone)]
pub struct AstTuplePattern {
    pub label: String,
    pub pattern: PatternId,
}


#[derive(Debug, Clone)]
pub struct AstMatchArm {
    pub pattern: PatternId,
    pub body: ExprId,
}

#[derive(Debug, Clone)]
pub struct AstEnumExpression {
    pub variant_name: Box<str>,
    pub attached_tuple: Option<ExprId>,
}