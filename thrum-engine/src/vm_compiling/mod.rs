use std::collections::HashMap;

use derive_more::Display;

use crate::{ErrType, lexing::tokens::{AssignOp, TokenKind}, parsing::ast::{AstArena, AstValue, Expr, ExprId, Pattern, PatternId}, pretty_printing::slice_to_string, typing::{ResolvedMemberAccess, ResolvedTypeInstantiation, TypeArena, TypeId, TypeVarId, TypedAst, type_vars::TypeVarConstVal}, vm_evaluating};


#[derive(Debug, Display, Clone, PartialEq, PartialOrd)]
pub enum VmValue {
    #[display("{_0}")]
    Int(i64),
    #[display("{_0:?}")]
    Float(f64),
    #[display("\"{_0}\"")]
    Str(String),
    #[display("{_0}")]
    Bool(bool),

    #[display("({})", slice_to_string(_0, ", "))]
    Tup(Vec<Self>),

    // raw unsafe pointer for the vm
    // this SHOULD be fully safe, since the borrow checker checked all the lifetimes.
    #[display("*<{_0:?}>")]
    ValuePointer(*mut Self),
    #[display("nativeFn<{_0:?}>")]
    NativeFn(fn(&[Self]) -> Result<Self, ErrType>),
    #[display("fn<{slot}>")]
    Fn { slot: usize },

    // for functions that return nothing
    #[display("<void>")]
    Void,

    // for the typechecker
    #[display("<{_0:?}>")]
    Type(TypeId),

    // for empty local slots in the vm
    // i could also use <void> here, but i want to be more clear
    #[display("<empty>")]
    Empty,
}

impl From<AstValue> for VmValue {
    fn from(value: AstValue) -> Self {
        match value {
            AstValue::NumInt(v) => Self::Int(v),
            AstValue::NumFloat(v) => Self::Float(v),
            AstValue::Str(v) => Self::Str(v),
            AstValue::Bool(v) => Self::Bool(v),
        }
    }
}


#[derive(Debug)]
pub enum OpCode {
    // Data Access
    ConstGet { const_index: usize },
    ConstGetRef { const_index: usize },
    PushVoid,

    // Temps
    ValuePop,
    ValueDup,

    // Locals
    LocalSet { local_index: usize },
    LocalPointer { local_index: usize },

    // Pointers
    PointerGetClone,
    PointerGetMove,
    PointerSet,

    // Math & Logic
    CmpEqual, CmpLess, CmpGreater,
    NumAdd { num_mode: NumMode }, NumSubtract { num_mode: NumMode },
    NumMultiply { num_mode: NumMode }, NumDivide { num_mode: NumMode },
    NumModulo { num_mode: NumMode }, NumNegate { num_mode: NumMode },
    BoolNegate,

    // Tuples
    TupCreate { length: usize },
    TupArrCreate { length: usize },
    TupPointerGet { index: usize },
    TupGet { index: usize },
    TupPointerIndex,
    TupUnpack { length: usize },

    // Strings
    StrAdd,
    StrTemplate { length: usize },

    /// these pop: [`TargetStr`]
    /// and push back: [`RemainingStr`, true] or [`TargetStr`, false]
    StrTrimPrefix { const_str: usize },
    StrTrimSuffix { const_str: usize },

    /// and push back: [`RemainingStr`, `ExtractedHoleStr`, true] or [`TargetStr`, `TargetStr`, false]
    StrTrimUntil { const_str: usize },

    // Control Flow
    Jump { offset: isize },
    JumpIfFalse { offset: isize },

    CallFn { arg_count: usize },

    // End of function / program
    Return,
    Panic,

    // meta type stuff
    MakeTypeRef { mutable: bool },
    TypeTupToTupType { labels: Box<[String]> },

    // no-op
    NoOp
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumMode {
    Int, Float
}

/// ### There are 2 Purposes for counting temps while generating `ByteCode`:
/// 1. Every expression HAS to produce exactly 1 temp.
///    if 2 + 2 were to produce 2 temps because of some bug, the compiler notices and panics.
/// 2. nested breaks/returns need to know how many temps to pop.
///    e.g. [1, 2, 3, "bla" + return]
struct OpCodeRuntimeTempDiff { requires: usize, diff: isize }
impl OpCode {
    fn runtime_temp_effect(&self) -> OpCodeRuntimeTempDiff {
        match self {
            Self::ConstGet { .. } | Self::ConstGetRef { .. }
            | Self::PushVoid | Self::LocalPointer { .. } => OpCodeRuntimeTempDiff { requires: 0, diff: 1 },

            Self::ValuePop | Self::LocalSet { .. } | Self::JumpIfFalse { .. } => OpCodeRuntimeTempDiff { requires: 1, diff: -1 },

            Self::ValueDup | Self::StrTrimPrefix { const_str: _ }
            | Self::StrTrimSuffix { const_str: _ } => OpCodeRuntimeTempDiff { requires: 1, diff: 1 },
            Self::StrTrimUntil { const_str: _ } => OpCodeRuntimeTempDiff { requires: 1, diff: 2 },

            Self::PointerSet => OpCodeRuntimeTempDiff { requires: 2, diff: -2 },

            Self::PointerGetClone | Self::PointerGetMove
            | Self::TupPointerGet { .. } | Self::TupGet { .. } | Self::TupArrCreate { length: _ }
            | Self::NumNegate { num_mode: _ } | Self::BoolNegate
            | Self::MakeTypeRef { .. }
            | Self::Return | Self::Panic
            | Self::TypeTupToTupType { labels: _ } => OpCodeRuntimeTempDiff { requires: 1, diff: 0 },

            Self::CmpEqual | Self::CmpLess | Self::CmpGreater
            | Self::TupPointerIndex
            | Self::NumAdd { num_mode: _ } | Self::NumSubtract { num_mode: _ }
            | Self::NumMultiply { num_mode: _ } | Self::NumDivide { num_mode: _ } | Self::NumModulo { num_mode: _ }
            | Self::StrAdd => OpCodeRuntimeTempDiff { requires: 2, diff: -1 },

            Self::StrTemplate { length }
            | Self::TupCreate { length } => OpCodeRuntimeTempDiff { requires: *length, diff: 1 - isize::try_from(*length).unwrap() },

            Self::TupUnpack { length } => OpCodeRuntimeTempDiff { requires: 1, diff: isize::try_from(*length).unwrap() - 1 },
            Self::Jump { .. } | Self::NoOp => OpCodeRuntimeTempDiff { requires: 0, diff: 0 },
            Self::CallFn { arg_count } => OpCodeRuntimeTempDiff { requires: *arg_count, diff: -isize::try_from(*arg_count).unwrap() },
        }
    }
}


#[derive(Debug, Clone, Copy)]
pub struct LabelId(pub usize);

#[derive(Default, Debug)]
pub struct BytecodeChunk {
    pub ops: Vec<OpCode>,
    pub constants: Vec<VmValue>,
    pub label_positions: Vec<LabelId>,
    pub local_slots_needed: usize,
}


enum CompilerVar {
    AtSlot(usize),
    ConstValue(VmValue)
}

struct FailureJump {
    jump_loc: usize,
    temps: usize,
}

pub struct VmCompiler<'a> {
    ast: &'a AstArena,
    typed_ast: &'a TypedAst,

    // bytecodes[curr_bytecode_index] is what this CompileFunction writes to.
    // the others are from other functions being compiled by other CompileFunction's.
    compiled_functions: &'a mut FunctionRegistry,
    curr_function_chunk: BytecodeChunk,

    locals: HashMap<TypeVarId, CompilerVar>,
    cur_function_var_amount: usize,

    // compiling `let x = 2 + 2`
    // - 2 temps (2, 2)
    // -> 1 temp (4)
    // -> 0 temps, because 4 got moved into a var slot (+1 var)
    // -> 1 temp, because the let-expr pushed void
    cur_temp_amount: usize,

    // break / continue
    loop_infos: HashMap<ExprId, LoopInfo>,
}

#[derive(Debug)]
pub enum CompilingStatus {
    NotCompiled,
    Compiling,
    Compiled(BytecodeChunk)
}
impl CompilingStatus {
    #[must_use]
    pub const fn not_compiled(&self) -> bool {
        matches!(self, Self::NotCompiled)
    }
}
#[derive(Debug)]
pub struct FunctionRegistry {
    pub compiled_functions: Vec<CompilingStatus>,
    pub closure_expr_id: Vec<ExprId>,
}
impl FunctionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self { compiled_functions: vec![CompilingStatus::NotCompiled], closure_expr_id: vec![ExprId(0)] }
    }
    pub fn reserve_slot(&mut self, closure_expr: ExprId) -> usize {
        let slot = self.compiled_functions.len();
        self.compiled_functions.push(CompilingStatus::NotCompiled);  // not compiled yet
        self.closure_expr_id.push(closure_expr);
        slot
    }
}

struct LoopInfo {
    // the end isn't known until the full loop is compiled.
    // so all break jumps temporarily hang out here
    break_jumps: Vec<usize>,
    temps_before: usize,
    // where the loop starts
    start_for_continue: usize,
}

impl VmCompiler<'_> {
    pub fn start(ast: &AstArena, typed_ast: &TypedAst, compiled_functions: &mut FunctionRegistry) {
        let mut vm_compiler = VmCompiler::new(ast, typed_ast, compiled_functions);

        vm_compiler.compile_and_insert_function(ExprId(0), &[], 0);
    }

    pub fn compile_and_run_comptime_expr(
        ast: &AstArena,
        typed_ast: &TypedAst,
        type_arena: &mut TypeArena,
        compiled_functions: &mut FunctionRegistry,
        expr_id: ExprId
    ) -> Result<VmValue, ErrType> {
        let const_chunk = VmCompiler::new(ast, typed_ast, compiled_functions)
            .compile_function(expr_id, &[]);

        // now just run it
        let mut reg = FunctionRegistry::new();
        reg.compiled_functions[0] = CompilingStatus::Compiled(const_chunk);
        println!("\n{reg:?}");
        unsafe { vm_evaluating::VM::start(&mut reg, Some(type_arena)) }
    }

    fn new<'a>(ast: &'a AstArena, typed_ast: &'a TypedAst, compiled_functions: &'a mut FunctionRegistry) -> VmCompiler<'a> {
        VmCompiler {
            ast, typed_ast,
            compiled_functions,
            curr_function_chunk: BytecodeChunk::default(),
            locals: HashMap::new(),
            cur_function_var_amount: 0,
            cur_temp_amount: 0,
            loop_infos: HashMap::new(),
        }
    }

    fn compile_and_insert_function(&mut self, expr: ExprId, params: &[PatternId], slot: usize) {
        assert!(self.compiled_functions.compiled_functions[slot].not_compiled(), "was already compiled??");
        self.compiled_functions.compiled_functions[slot] = CompilingStatus::Compiling;

        let fn_chunk = self.compile_function(expr, params);
        self.compiled_functions.compiled_functions[slot] = CompilingStatus::Compiled(fn_chunk);
    }

    fn compile_function(&mut self, expr: ExprId, params: &[PatternId]) -> BytecodeChunk {
        let backup = (std::mem::take(&mut self.curr_function_chunk), self.cur_temp_amount, self.cur_function_var_amount);
        // set the current compiling function to the slot we want to compile.
        // this puts None in that slot in `self.compiled_functions`.
        self.curr_function_chunk = BytecodeChunk::default();
        self.cur_temp_amount = params.len();
        self.cur_function_var_amount = 0;

        // compile the params first
        for &param in params.iter().rev() {
            self.compile_binding_pattern(param, &mut Vec::new());
        }

        // then compile the body!!
        self.compile_expression(expr);

        // finally, return
        self.push_op(OpCode::Return);

        let new_chunk = std::mem::take(&mut self.curr_function_chunk);

        (self.curr_function_chunk, self.cur_temp_amount, self.cur_function_var_amount) = backup;

        new_chunk
    }


    fn compile_expression(&mut self, compile_expr: ExprId) {
        let start_temps = self.cur_temp_amount;
        let expr = self.ast.get_expr(compile_expr);

        match expr {
            Expr::Void => {
                self.push_op(OpCode::PushVoid);
            }
            Expr::Literal { val } => {
                self.push_get_constant_op(VmValue::from(val.clone()));
            }
            Expr::TemplateString { elems } => {
                for &elem in elems {
                    self.compile_expression(elem);
                }
                self.push_op(OpCode::StrTemplate { length: elems.len() });
            }
            Expr::TupleArr { elem, length: _ } => {
                let const_length = self.typed_ast.resolved_tuple_arr_length[&compile_expr];
                if const_length == 0 {
                    self.push_op(OpCode::TupCreate { length: 0 });
                } else {
                    self.compile_expression(*elem);
                    self.push_op(OpCode::TupArrCreate { length: const_length });
                }
            }
            Expr::Tuple { elems } => {
                for elem in elems {
                    self.compile_expression(elem.expr);
                }
                self.push_op(OpCode::TupCreate { length: elems.len() });
            }


            Expr::Infix { op, op_span: _, left, right } => {
                self.compile_expression(*left);
                self.compile_infix(*op, *right, compile_expr);
            }
            Expr::Prefix { op, right } => {
                self.compile_prefix(*op, *right, compile_expr);
            }

            Expr::Block { exprs, label } => {
                // process all other expressions
                if let Some((&last_expr, not_last_exprs)) = exprs.split_last() {
                    // block label logic
                    if label.is_some() {
                        self.loop_infos.insert(compile_expr, LoopInfo {
                            break_jumps: Vec::new(),
                            temps_before: self.cur_temp_amount,
                            start_for_continue: usize::MAX
                        });
                    }

                    // actual compiling of the expressions
                    for &expr in not_last_exprs {
                        self.compile_expression(expr);
                        // if it is not the last expression of the block, the return value is discarded, so pop it.
                        // this will mostly get optimized out later
                        self.push_op(OpCode::ValuePop);
                    }
                    self.compile_expression(last_expr);


                    // all break jumps that need to jump to the end of this block.
                    if label.is_some() {
                        let loop_info = self.loop_infos.remove(&compile_expr).unwrap();

                        for jump in loop_info.break_jumps {
                            self.patch_jump_op_to_here(jump);
                        }
                    }
                }
                else {
                    // if block is empty, just push void
                    self.push_op(OpCode::PushVoid);
                }
            }

            Expr::IdentifierRef { .. } => {
                let var_id = *self.typed_ast.resolved_expr_var.get(&compile_expr)
                    .unwrap_or_else(|| panic!("{:?}", self.ast.get_expr(compile_expr)));
                self.push_get_identifier_ref(var_id);
            }

            Expr::Assign { pattern, value, extra_op, op_span: _ } => {
                if let Some(extra_op) = extra_op {
                    // example: x += 2
                    // it needs to first push the value of x, compute x + 2, then set x to that result
                    match self.ast.get_pattern(*pattern) {
                        Pattern::PlacePointer(expr) => {
                            self.compile_expression(*expr);
                            self.push_op(OpCode::PointerGetMove);
                        }
                        _ => unreachable!("Infix assignments are only allowed for place patterns.")
                    }
                    self.compile_infix(TokenKind::Op(*extra_op), *value, compile_expr);
                } else {
                    // push value to the stack
                    self.compile_expression(*value);
                }

                // compile the binding pattern
                self.compile_binding_pattern(*pattern, &mut Vec::new());

                // every assign expression results in void
                self.push_op(OpCode::PushVoid);
            }

            Expr::EmptyLet { pattern } => {
                // no value, just define the variables
                let var_id = self.typed_ast.resolved_pattern_var[pattern];
                self.define_local(var_id, None);

                self.push_op(OpCode::PushVoid);
            }

            Expr::Is { value, pattern } => {
                self.compile_expression(*value);

                let mut failure_jumps = Vec::new();
                self.compile_binding_pattern(*pattern, &mut failure_jumps);

                // it matched! -> push true
                let temps_before_pushing_true = self.cur_temp_amount;
                self.push_get_constant_op(VmValue::Bool(true));

                // handle failure path
                if !failure_jumps.is_empty() {
                    // reset temps to handle the false path
                    self.cur_temp_amount = temps_before_pushing_true;

                    // jump over this false path if it matched
                    let jump_over_false_path = self.push_jump_op_for_patching();

                    // all failure jumps land here
                    self.compile_binding_pattern_failure_jumps(failure_jumps);
                    self.push_get_constant_op(VmValue::Bool(false));

                    self.patch_jump_op_to_here(jump_over_false_path);
                }
            }

            Expr::Move { expr } => {
                self.compile_expression(*expr);
                if self.typed_ast.move_expr.contains(&compile_expr) {
                    self.push_op(OpCode::PointerGetMove);
                } else {
                    self.push_op(OpCode::PointerGetClone);
                }
            }


            Expr::If { condition, then, alt, never_alt: _ } => {
                self.compile_expression(*condition);
                let jump_to_else_block = self.push_jump_if_false_op_for_patching();

                // if we are here, the condition matched
                let temps_before_consequence = self.cur_temp_amount;
                self.compile_expression(*then);
                let jump_over_else_block = self.push_jump_op_for_patching();

                self.patch_jump_op_to_here(jump_to_else_block);

                // if we are here the condition failed
                self.cur_temp_amount = temps_before_consequence;
                self.compile_expression(*alt);

                self.patch_jump_op_to_here(jump_over_else_block);
            }

            Expr::Loop { body, label: _ } => {
                // what it should look like:
                // ...loop...
                // ...loop...
                // Jump -3
                let op_vec = &mut self.curr_function_chunk.ops;
                if op_vec.len() < 2 {
                    op_vec.resize_with(2, || OpCode::NoOp);
                }
                let loop_start = op_vec.len() - 2;
                self.loop_infos.insert(compile_expr, LoopInfo {
                    break_jumps: Vec::new(),
                    temps_before: self.cur_temp_amount,
                    start_for_continue: loop_start
                });

                // ----- Main Loop Logic -----
                self.compile_expression(*body);
                self.push_op(OpCode::ValuePop);
                // and loop back
                self.push_backwards_jump_op(loop_start);
                // ---------------------------

                // all break/continue jumps that are refering to this loop.
                let loop_info = self.loop_infos.remove(&compile_expr).unwrap();

                for jump in loop_info.break_jumps {
                    self.patch_jump_op_to_here(jump);
                }

                // doesnt need to push void, because it's an infinite loop
                self.cur_temp_amount += 1;
            }

            Expr::Break { expr, label: _ } => {
                let break_to = self.typed_ast.resolved_labels[&compile_expr];
                let loop_info = &self.loop_infos[&break_to];
                let temp_pop_amount = self.cur_temp_amount - loop_info.temps_before;

                // pop required temp amount
                // this preserves any temps before this loop
                // e.g. horrendous code like this works: `1 + loop { 1 + break 1 }` -> 2
                for _ in 0..temp_pop_amount {
                    self.push_op(OpCode::ValuePop);
                }
                // then compile the break expression
                self.compile_expression(*expr);

                // and then actually break
                let break_jump = self.push_jump_op_for_patching();

                // find the correct loop to break to
                self.loop_infos.get_mut(&break_to).unwrap().break_jumps.push(break_jump);

                // pretend like this break expression didn't happen, to compile expressions after this.
                self.cur_temp_amount += temp_pop_amount;
            }

            Expr::Continue { .. } => {
                let break_to = self.typed_ast.resolved_labels[&compile_expr];
                let loop_info = &self.loop_infos[&break_to];
                let temp_pop_amount = self.cur_temp_amount - loop_info.temps_before;
                let start_for_continue = loop_info.start_for_continue;

                // pop required temp amount
                for _ in 0..temp_pop_amount {
                    self.push_op(OpCode::ValuePop);
                }

                // find the correct loop to continue to
                self.push_backwards_jump_op(start_for_continue);

                // pretend like this continue expression didn't happen, to compile expressions after this.
                self.cur_temp_amount += temp_pop_amount + 1;
            }


            Expr::Index { left, index } => {
                self.compile_expression(*left);
                self.compile_expression(*index);
                self.push_op(OpCode::TupPointerIndex);
            }


            Expr::Match { match_value, arms } => {
                self.compile_expression(*match_value);
                let mut success_jumps = Vec::new();
                let mut prev_failure_jumps = Vec::new();

                for (i, arm) in arms.iter().enumerate() {
                    let is_last = i + 1 == arms.len();

                    // on the first iteration this does nothing
                    self.compile_binding_pattern_failure_jumps(prev_failure_jumps);
                    prev_failure_jumps = Vec::new();

                    // duplicate the to match value
                    if !is_last { self.push_op(OpCode::ValueDup); }
                    self.compile_binding_pattern(arm.pattern, &mut prev_failure_jumps);

                    // if it matched -> pop original to match value
                    if !is_last { self.push_op(OpCode::ValuePop); }
                    self.compile_expression(arm.body);
                    success_jumps.push(self.push_jump_op_for_patching());
                }

                // ignore the final `prev_failure_jumps` because the last arm can't fail

                // end of match statement, point all arm-body-success-jumps here
                for jump in success_jumps {
                    self.patch_jump_op_to_here(jump);
                }
            }

            Expr::Call { callee, arguments } => {
                let mut arg_count = 0;
                if let Some(ResolvedMemberAccess::MemberWithSelfSugar { self_sugar_expr, .. }) = self.typed_ast.resolved_member_access.get(callee) {
                    self.compile_expression(*self_sugar_expr);
                    arg_count += 1;
                }

                for &argument in arguments {
                    self.compile_expression(argument);
                    arg_count += 1;
                }
                self.compile_expression(*callee);
                self.push_op(OpCode::CallFn { arg_count });
            }

            Expr::TypeInstantiation { typ: _, data } => {
                match self.typed_ast.resolved_type_instantian[&compile_expr] {
                    ResolvedTypeInstantiation::NewType => {
                        // e.g. `N{ 2 }`
                        let Expr::Tuple { elems } = self.ast.get_expr(*data) else {
                            unreachable!("always a tuple.")
                        };
                        self.compile_expression(elems[0].expr);
                    }
                    ResolvedTypeInstantiation::Tuple => {
                        self.compile_expression(*data);
                    }
                    ResolvedTypeInstantiation::EnumVariant(i) => {
                        self.compile_enum_variant(i, Some(*data));
                    }
                }
            }

            Expr::Return { expr } => {
                let temp_pop_amount = self.cur_temp_amount;

                // pop all temp values
                for _ in 0..temp_pop_amount {
                    self.push_op(OpCode::ValuePop);
                }
                // then compile the break expression
                self.compile_expression(*expr);
                self.push_op(OpCode::Return);

                self.cur_temp_amount += temp_pop_amount;
            }

            Expr::MemberAccess { left, member: _ } => {
                match &self.typed_ast.resolved_member_access[&compile_expr] {
                    ResolvedMemberAccess::TupleRefIndex { index } => {
                        self.compile_expression(*left);
                        self.push_op(OpCode::TupPointerGet { index: *index });
                    }
                    ResolvedMemberAccess::TupleIndex { index } => {
                        self.compile_expression(*left);
                        self.push_op(OpCode::TupGet { index: *index });
                    }
                    ResolvedMemberAccess::Member { constant }
                    | ResolvedMemberAccess::MemberWithSelfSugar { constant, .. } => {
                        self.push_get_constant_ref_op(constant.clone());
                    }
                    ResolvedMemberAccess::EnumWithNoData { i } => {
                        self.compile_enum_variant(*i, None);
                    }
                }
            }


            Expr::EnumVariant { data } => {
                let (_enum_id, i) = self.typed_ast.resolved_enum_variant[&compile_expr];
                self.compile_enum_variant(i, data.attached_tuple);
            }

            Expr::ImplSelf => {
                let meta_type = self.typed_ast.resolved_impl_self_type[&compile_expr];
                self.push_get_constant_op(VmValue::Type(meta_type));
            }

            Expr::Borrow { expr, mutable } => {
                self.compile_expression(*expr);
                self.push_op(OpCode::MakeTypeRef { mutable: *mutable });
            }

            // do nothing here, these are only for the typechecker.
            Expr::Const { .. }
            | Expr::CustomType { .. }
            | Expr::ImplBlock { .. } => self.push_op(OpCode::PushVoid),

            _ => panic!("{expr:?} not yet implemented")
        }

        if let Some(&deref_amount) = self.typed_ast.auto_derefs.get(&compile_expr) {
            for _ in 0..deref_amount {
                // println!("derefed {:?}", self.ast.get_expr(compile_expr));
                self.push_op(OpCode::PointerGetClone);
            }
        }

        if let Some(labels) = self.typed_ast.resolved_tuple_type_coerce.get(&compile_expr) {
            self.push_op(OpCode::TypeTupToTupType { labels: labels.clone() });
        }

        assert!(start_temps + 1 == self.cur_temp_amount,
            "wrong temp number ({}, should be {}) after processing {:?}", self.cur_temp_amount, start_temps + 1, expr);
    }





    #[track_caller]
    fn push_op(&mut self, op: OpCode) {
        let effect = op.runtime_temp_effect();
        assert!(effect.requires <= self.cur_temp_amount,
            "Compiler does not have enough temps ({}) to push_op() {op:?}",
            self.cur_temp_amount
        );
        self.cur_temp_amount = self.cur_temp_amount.strict_add_signed(effect.diff);
        self.curr_function_chunk.ops.push(op);
    }


    fn push_jump_op_for_patching(&mut self) -> usize {
        self.push_op(OpCode::Jump { offset: isize::MAX });
        self.curr_function_chunk.ops.len() - 1
    }
    fn push_jump_if_false_op_for_patching(&mut self) -> usize {
        self.push_op(OpCode::JumpIfFalse { offset: isize::MAX });
        self.curr_function_chunk.ops.len() - 1
    }

    fn patch_jump_op_to_here(&mut self, to_patch_location: usize) {
        let codes = &mut self.curr_function_chunk.ops;
        let codes_len = codes.len();
        match &mut codes[to_patch_location] {
            OpCode::Jump { offset }
            | OpCode::JumpIfFalse { offset } => {
                *offset = isize::try_from(codes_len - to_patch_location - 1).unwrap();
            }
            not_a_jump => panic!("{not_a_jump:?} is not a jump instruction, so it can't be patched.")
        }
    }

    fn push_backwards_jump_op(&mut self, destination: usize) {
        self.push_op(OpCode::Jump {
            offset: isize::try_from(destination).unwrap()
                .strict_sub_unsigned(self.curr_function_chunk.ops.len()) + 1
        });
    }


    fn add_constant(&mut self, val: VmValue) -> usize {
        // let index = match self.bytecode.constants.iter().position(|constant| val == constant.clone()) {
        //     Some(i) => i,
        //     None => {
        //         self.bytecode.constants.push(val);
        //         self.bytecode.constants.len() - 1
        //     }
        // };
        self.curr_function_chunk.constants.push(val);
        self.curr_function_chunk.constants.len() - 1
    }

    fn push_get_constant_op(&mut self, val: VmValue) {
        let const_index = self.add_constant(val);
        self.push_op(OpCode::ConstGet { const_index });
    }
    fn push_get_constant_ref_op(&mut self, val: VmValue) {
        // if the const value is a fn that is not compiled yet, compile it!
        if let VmValue::Fn { slot } = val
        && self.compiled_functions.compiled_functions[slot].not_compiled() {

            let closure_expr = self.compiled_functions.closure_expr_id[slot];
            let Expr::Closure { closure, .. } = self.ast.get_expr(closure_expr) else {
                unreachable!("woopsie?")
            };
            self.compile_and_insert_function(closure.body, &closure.params, slot);
        }

        let const_index = self.add_constant(val);
        self.push_op(OpCode::ConstGetRef { const_index });
    }

    fn define_local(&mut self, var_id: TypeVarId, const_value: Option<VmValue>) -> usize {
        // doesn't exist yet, define it!
        let slot = self.cur_function_var_amount;
        self.cur_function_var_amount += 1;

        self.locals.insert(var_id, const_value.map_or(CompilerVar::AtSlot(slot), CompilerVar::ConstValue));

        // if this is the highest amount of vars needed so far, store that
        if self.cur_function_var_amount > self.curr_function_chunk.local_slots_needed {
            self.curr_function_chunk.local_slots_needed = self.cur_function_var_amount;
        }

        slot
    }

    fn push_define_local(&mut self, var_id: TypeVarId) {
        let local_index = self.define_local(var_id, None);
        self.push_op(OpCode::LocalSet { local_index });
    }

    fn push_get_identifier_ref(&mut self, var_id: TypeVarId) {
        match self.locals.get(&var_id) {
            None => {
                match &self.typed_ast.get_var(var_id).const_val {
                    TypeVarConstVal::Evaluated(val) => {
                        self.push_get_constant_ref_op(val.clone());
                    }
                    TypeVarConstVal::NotYetEvaluated { value, bind_to } => {
                        unreachable!("const was not evaluated yet... {:?} {bind_to:?}", self.ast.display_expr(*value))
                    }
                    _ => unreachable!("{var_id:?} is not in the current variables..."),
                }
            }
            Some(CompilerVar::ConstValue(val)) => {
                self.push_get_constant_ref_op(val.clone());
            }
            Some(CompilerVar::AtSlot(slot)) => {
                self.push_op(OpCode::LocalPointer { local_index: *slot });
            }
        }
    }







    fn compile_binding_pattern(&mut self, compile_pattern: PatternId, failure_jumps: &mut Vec<FailureJump>) {
        let start_temps = self.cur_temp_amount;
        let pattern = self.ast.get_pattern(compile_pattern);

        match pattern {
            Pattern::Wildcard => self.push_op(OpCode::ValuePop),

            Pattern::Binding { .. } => {
                let var_id = self.typed_ast.resolved_pattern_var[&compile_pattern];
                self.push_define_local(var_id);
            }

            Pattern::Tuple(patterns) => {
                // manually do stuff that self.push_op() does
                // because OpCode::runtime_temp_effect can't know how many temps TupUnpack is gonna add.
                self.push_op(OpCode::TupUnpack { length: patterns.len() });

                for pattern in patterns.iter().rev() {
                    self.compile_binding_pattern(pattern.pattern, failure_jumps);
                }
            }

            Pattern::Or(patterns) => {
                let mut prev_failure_jumps = Vec::new();
                let mut success_jumps = Vec::new();

                for (i, pat) in patterns.iter().enumerate() {
                    let is_last = i + 1 == patterns.len();

                    // no prev_failure_jumps the first time, so nothing happens
                    self.compile_binding_pattern_failure_jumps(prev_failure_jumps);
                    prev_failure_jumps = Vec::new();

                    // dupe the value each time, so it doesn't get used up.
                    if !is_last { self.push_op(OpCode::ValueDup); }
                    self.compile_binding_pattern(*pat, &mut prev_failure_jumps);

                    // if this point is reached an or pattern matched!
                    success_jumps.push(self.push_jump_op_for_patching());
                }

                failure_jumps.extend(prev_failure_jumps);

                if let [others @ .., last] = success_jumps.as_slice() {
                    // point all `success_jumps` to here
                    for sj in others {
                        self.patch_jump_op_to_here(*sj);
                    }

                    // now that it matched, pop the original value
                    self.cur_temp_amount += 1;
                    self.push_op(OpCode::ValuePop);

                    // which isn't needed for the last pattern, because it was never Duped in the first place
                    self.patch_jump_op_to_here(*last);
                } else {
                    unreachable!("Or-patterns should have at least one pattern...")
                }
            }

            Pattern::PlacePointer(expr) => {
                self.compile_expression(*expr);
                self.push_op(OpCode::PointerSet);
            }

            // Failable patterns
            Pattern::CompareExpr(expr) => {
                self.compile_expression(*expr);
                self.push_op(OpCode::CmpEqual);
                failure_jumps.push(FailureJump {
                    jump_loc: self.push_jump_if_false_op_for_patching(),
                    temps: self.cur_temp_amount
                });
            }

            Pattern::Conditional { pattern, cond } => {
                self.compile_binding_pattern(*pattern, failure_jumps);
                self.compile_expression(*cond);
                failure_jumps.push(FailureJump {
                    jump_loc: self.push_jump_if_false_op_for_patching(),
                    temps: self.cur_temp_amount
                });
            }

            Pattern::TypeDestructor { typ: _, data } => {
                if self.typed_ast.resolved_type_destruction_not_a_tuple.contains(&compile_pattern) {
                    // if its not a tuple type, e.g. `type N = num; N{ 2 }`
                    let Pattern::Tuple(elems) = self.ast.get_pattern(*data) else {
                        unreachable!("always a tuple.")
                    };
                    self.compile_binding_pattern(elems[0].pattern, failure_jumps);
                }
                else {
                    self.compile_binding_pattern(*data, failure_jumps);
                }
            }

            Pattern::EnumVariant { name: _, attached_tuple } => {
                let (_enum_id, i) = self.typed_ast.resolved_enum_variant_pattern[&compile_pattern];

                self.push_op(OpCode::TupUnpack { length: 2 });

                // compare the enum tags
                self.push_get_constant_op(VmValue::Int(i.try_into().unwrap()));
                self.push_op(OpCode::CmpEqual);
                failure_jumps.push(FailureJump {
                    jump_loc: self.push_jump_if_false_op_for_patching(),
                    temps: self.cur_temp_amount
                });

                // if the enum tags were equal, compare the data
                if let Some(tup) = attached_tuple {
                    self.compile_binding_pattern(*tup, failure_jumps);
                } else {
                    self.push_op(OpCode::ValuePop);
                }
            }

            Pattern::String { before, hole_parts } => {
                if hole_parts.is_empty() {
                    self.push_get_constant_op(VmValue::Str(before.clone()));
                    self.push_op(OpCode::CmpEqual);
                    failure_jumps.push(FailureJump {
                        jump_loc: self.push_jump_if_false_op_for_patching(),
                        temps: self.cur_temp_amount
                    });
                }
                else {
                    let before_id = self.add_constant(VmValue::Str(before.clone()));
                    self.push_op(OpCode::StrTrimPrefix { const_str: before_id });
                    failure_jumps.push(FailureJump {
                        jump_loc: self.push_jump_if_false_op_for_patching(),
                        temps: self.cur_temp_amount
                    });

                    for (i, (hole_pat, hole_after)) in hole_parts.iter().enumerate() {
                        let is_last_hole = i == hole_parts.len() - 1;

                        let after_id = self.add_constant(VmValue::Str(hole_after.clone()));
                        if is_last_hole {
                            self.push_op(OpCode::StrTrimSuffix { const_str: after_id });
                        } else {
                            self.push_op(OpCode::StrTrimUntil { const_str: after_id });
                        }
                        failure_jumps.push(FailureJump {
                            jump_loc: self.push_jump_if_false_op_for_patching(),
                            temps: self.cur_temp_amount
                        });

                        self.compile_binding_pattern(*hole_pat, failure_jumps);
                    }
                }
            }

            Pattern::Not(pat) => {
                let mut good_failure_jumps = Vec::new();
                self.compile_binding_pattern(*pat, &mut good_failure_jumps);

                // if it matched => failure jump
                failure_jumps.push(FailureJump {
                    jump_loc: self.push_jump_op_for_patching(),
                    temps: self.cur_temp_amount
                });

                // if it didn't match its good!
                self.compile_binding_pattern_failure_jumps(good_failure_jumps);
            }


            Pattern::Typed { pattern, typ: _ } => {
                self.compile_binding_pattern(*pattern, failure_jumps);
            }
        }

        assert!(start_temps - 1 == self.cur_temp_amount,
            "wrong temp number ({}, should be {}) after processing {:?}", self.cur_temp_amount, start_temps - 1, pattern);
    }



    fn compile_binding_pattern_failure_jumps(&mut self, mut failure_jumps: Vec<FailureJump>) {
        // complicated function but it does this:
        // if one failure jump had 5 temps
        // and another had 3 temps
        // and we need to get to 2 temps
        // then it compiles to this:
        // pop <- temp5 jump lands here
        // pop
        // pop <- temp 3 jump lands here

        // sort biggest to smallest
        failure_jumps.sort_by_key(|b| std::cmp::Reverse(b.temps));

        let mut jumps_iter = failure_jumps.iter().peekable();

        while let Some(current_jump) = jumps_iter.next() {
            self.patch_jump_op_to_here(current_jump.jump_loc);

            // Determine the temp amount needed for the next failure jump
            let next_cleanup_depth = match jumps_iter.peek() {
                Some(next_jump) => next_jump.temps,
                // if this already was last jump, the next depth is the final target.
                None => self.cur_temp_amount,
            };

            // how many pops do we need to get from our current depth to the next?
            let pops_needed = current_jump.temps - next_cleanup_depth;
            for _ in 0..pops_needed {
                self.curr_function_chunk.ops.push(OpCode::ValuePop);
            }
        }
    }





    fn compile_infix(&mut self, operator: TokenKind, right: ExprId, expr_id: ExprId) {
        if TokenKind::EqualEqual == operator {
            // this works regardless of any type
            self.compile_expression(right);
            self.push_op(OpCode::CmpEqual);
            return
        }

        // short circuiting operators,
        // they need to compile `right` later.
        match operator {
            TokenKind::And => {
                // evaluate left
                // left is true => discard it and return right
                // left is false => return false
                let temps_after_left = self.cur_temp_amount;
                let jump_over_right_expression = self.push_jump_if_false_op_for_patching();
                self.compile_expression(right);
                let jump_to_end = self.push_jump_op_for_patching();
                self.patch_jump_op_to_here(jump_over_right_expression);
                self.push_get_constant_op(VmValue::Bool(false));
                self.cur_temp_amount = temps_after_left;
                self.patch_jump_op_to_here(jump_to_end);
                return
            }
            TokenKind::Or => {
                // evaluate left
                // left is true => return true
                // left is false => discard it and return right
                self.push_op(OpCode::BoolNegate);
                let temps_after_left = self.cur_temp_amount;
                let jump_over_right_expression = self.push_jump_if_false_op_for_patching();
                self.compile_expression(right);
                let jump_to_end = self.push_jump_op_for_patching();
                self.patch_jump_op_to_here(jump_over_right_expression);
                self.push_get_constant_op(VmValue::Bool(true));
                self.cur_temp_amount = temps_after_left;
                self.patch_jump_op_to_here(jump_to_end);
                return
            }
            _ => ()
        }

        // for normal operators just compile right
        self.compile_expression(right);

        let num_mode = *self.typed_ast.resolved_num_mode.get(&expr_id).unwrap_or(&NumMode::Int);

        match operator {
            TokenKind::Op(AssignOp::Plus) => self.push_op(OpCode::NumAdd { num_mode }),
            TokenKind::Op(AssignOp::Minus) => self.push_op(OpCode::NumSubtract { num_mode }),
            TokenKind::Op(AssignOp::Star) => self.push_op(OpCode::NumMultiply { num_mode }),
            TokenKind::Op(AssignOp::Slash) => self.push_op(OpCode::NumDivide { num_mode }),
            TokenKind::Op(AssignOp::Percent) => self.push_op(OpCode::NumModulo { num_mode }),
            TokenKind::Less => self.push_op(OpCode::CmpLess),
            TokenKind::Greater => self.push_op(OpCode::CmpGreater),

            _ => unreachable!("Unsupported operator: {operator}")
        }
    }

    fn compile_prefix(&mut self, operator: TokenKind, right: ExprId, expr_id: ExprId) {
        self.compile_expression(right);

        match operator {
            TokenKind::Exclamation => self.push_op(OpCode::BoolNegate),
            TokenKind::Op(AssignOp::Minus) => {
                let num_mode = *self.typed_ast.resolved_num_mode.get(&expr_id).unwrap_or(&NumMode::Int);
                self.push_op(OpCode::NumNegate { num_mode });
            }
            _ => unreachable!("unsupported prefix operator...")
        }
    }

    fn compile_enum_variant(&mut self, i: usize, data: Option<ExprId>) {
        // this compiles to a 2-tuple: (data, tag)
        if let Some(tup) = data {
            self.compile_expression(tup);
        } else {
            self.push_op(OpCode::PushVoid);
        }
        self.push_get_constant_op(VmValue::Int(i.try_into().unwrap()));
        self.push_op(OpCode::TupCreate { length: 2 });
    }
}