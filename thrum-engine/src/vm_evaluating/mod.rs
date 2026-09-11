use crate::{ErrType, pretty_printing::slice_to_string, typing::{Type, TypeArena, TypeTuple}, vm_compiling::{CompilingStatus, FunctionRegistry, NumMode, OpCode, VmValue}};



macro_rules! infix_op {
    ($self:expr, $left_pattern:pat, $right_pattern:pat => $result_expr:expr) => {{
        let right = $self.value_stack.pop().unwrap();
        let left = $self.value_stack.pop().unwrap();
        match (left, right) {
            // (Value::Num(l), Value::Num(r)) => self.stack_push_val(Value::Num(l + r))
            ($left_pattern, $right_pattern) => $self.stack_push_val($result_expr),
            (left, right) => unreachable!("Operands {left}, {right} cannot be '{}'-ed", stringify!($result_expr)),
        }
    }};
}
macro_rules! prefix_op {
    ($self:expr, $operand_pattern:pat => $result_expr:expr) => {{
        let operand = $self.value_stack.pop().unwrap();
        match operand {
            // Value::Num(n) => self.stack_push_val(Value::Num(-n))
            $operand_pattern => $self.stack_push_val($result_expr),
            operand => unreachable!("Operand {operand} cannot be '{}'-ed", stringify!($result_expr)),
        }
    }};
}


macro_rules! infix_num_op {
    ($self:expr, $mode:expr, $op:tt) => {{
        let right = $self.value_stack.pop().expect("Stack underflow");
        let left = $self.value_stack.last_mut().expect("Stack underflow");

        #[expect(clippy::assign_op_pattern, reason="idk why clippy lints on this...")]
        match ($mode, left, right) {
            (NumMode::Int, VmValue::Int(l), VmValue::Int(r)) => *l = *l $op r,
            (NumMode::Float, VmValue::Float(l), VmValue::Float(r)) => *l = *l $op r,
            (_, left, right) => unreachable!("Type mismatch: ({:?}) {left} {} {right}", $mode, stringify!($op)),
        }
    }};
}
macro_rules! prefix_num_op {
    ($self:expr, $mode:expr, $op:tt) => {{
        let right = $self.value_stack.last_mut().expect("Stack underflow");

        match ($mode, right) {
            (NumMode::Int, VmValue::Int(r)) => *r = $op *r,
            (NumMode::Float, VmValue::Float(r)) => *r = $op *r,
            (_, right) => unreachable!("Type mismatch: ({:?}) {} {right}", $mode, stringify!($op)),
        }
    }};
}




// a new CallFrame is pushed when:
// - entering a function
#[derive(Default)]
pub struct CallFrame {
    // points to the current BytecodeChunk we are executing
    chunk_index: usize,

    // instruction pointer
    pub ip: usize,

    // the current location in the stack where temp values are being calculated with
    //                                                          v points here
    // [... values from functions lower on the callstack ..., LOCAL1, LOCAL2, LOCAL3, TEMP1, TEMP2 TEMP3]
    base_pointer: usize,
}



pub struct VM<'a> {
    compiled_functions: &'a mut FunctionRegistry,
    // for evaluating consts it needs to be able
    // to add new types and get TypeId's
    type_arena: Option<&'a mut TypeArena>,

    frames: Vec<CallFrame>,
    value_stack: Vec<VmValue>,
}

impl<'a> VM<'a> {
    pub unsafe fn start(compiled_functions: &'a mut FunctionRegistry, type_arena: Option<&'a mut TypeArena>) -> Result<VmValue, ErrType> {
        let mut vm = Self {
            compiled_functions,
            type_arena,
            frames: Vec::new(),
            // this stack is NOT allowed to reallocate.
            // if it does, every Value::ValuePointer(*mut Value) breaks and unsafe behaviour happens :(
            value_stack: Vec::with_capacity(1024)
        };
        vm.load_frame_from_index(0);
        unsafe { vm.run(cfg!(debug_assertions)) }
    }


    fn load_frame_from_index(&mut self, chunk_index: usize) {
        let CompilingStatus::Compiled(chunk) = &self.compiled_functions.compiled_functions[chunk_index] else {
            panic!("function {chunk_index} was not compiled...")
        };
        let frame = CallFrame {
            chunk_index,
            ip: 0,
            base_pointer: self.value_stack.len(),
        };
        let length_needed = frame.base_pointer + chunk.local_slots_needed;
        // self.value_stack.resize_with(length_needed, || Value::Empty);
        while self.value_stack.len() < length_needed {
            self.stack_push_val(VmValue::Empty);
        }
        self.frames.push(frame);
    }

    /// # Safety
    /// The typechecker should make sure that `ValuePointers` are used correctly!
    /// If it fails, this will cause UB. :(
    pub unsafe fn run(&mut self, print_debug_execution: bool) -> Result<VmValue, ErrType> {
        loop {
            let Some(frame) = self.frames.last_mut() else {
                // else the program finished!
                let [final_val] = &self.value_stack[..] else {
                    unreachable!("not exactly 1 final value?! {:?}", self.value_stack)
                };
                return Ok(final_val.clone())
            };
            let CompilingStatus::Compiled(chunk) = &mut self.compiled_functions.compiled_functions[frame.chunk_index] else {
                panic!("function {} was not compiled...", frame.chunk_index)
            };
            let instruction = &chunk.ops[frame.ip];
            frame.ip += 1;

            if print_debug_execution {
                print!("{:<28}", format!("{instruction:?}"));
                // value stack will be printed after the match vv
            }
            match instruction {
                // Data Access
                OpCode::ConstGet { const_index } => {
                    let val = chunk.constants[*const_index].clone();
                    self.stack_push_val(val);
                }
                OpCode::ConstGetRef { const_index } => {
                    let pointer = &raw mut chunk.constants[*const_index];
                    self.stack_push_val(VmValue::ValuePointer(pointer));
                }
                OpCode::PushVoid => {
                    self.stack_push_val(VmValue::Void);
                }


                // Temps
                OpCode::ValuePop => {
                    self.value_stack.pop().unwrap();
                }
                OpCode::ValueDup => {
                    self.stack_push_val(
                        self.value_stack.last().unwrap().clone()
                    );
                }


                // Locals
                OpCode::LocalSet { local_index } => {
                    let value = self.value_stack.pop().unwrap();
                    self.value_stack[frame.base_pointer + local_index] = value;
                }
                OpCode::LocalPointer { local_index } => {
                    let pointer = &raw mut self.value_stack[frame.base_pointer + local_index];
                    self.stack_push_val(VmValue::ValuePointer(pointer));
                }


                // Pointers
                OpCode::PointerGetClone => {
                    let VmValue::ValuePointer(raw_p) = self.value_stack.pop().unwrap()
                    else { unreachable!() };

                    // unsafe, spooky
                    self.stack_push_val(
                        unsafe { &mut *raw_p }.clone()
                    );
                }
                OpCode::PointerGetMove => {
                    let VmValue::ValuePointer(raw_p) = self.value_stack.pop().unwrap()
                    else { unreachable!() };

                    // unsafe, spooky
                    self.stack_push_val(
                        std::mem::replace(unsafe { &mut *raw_p }, VmValue::Empty)
                    );
                }
                OpCode::PointerSet => {
                    let VmValue::ValuePointer(p) = self.value_stack.pop().unwrap()
                    else { unreachable!("Value was not a pointer") };
                    let value = self.value_stack.pop().unwrap();

                    unsafe { *p = value; }
                }

                // Math & Logic
                // relies on `impl PartialEq for Value`
                OpCode::CmpEqual => {
                    let right = self.value_stack.pop().unwrap();
                    let left = self.value_stack.pop().unwrap();
                    self.stack_push_val(VmValue::Bool(left == right));
                }
                OpCode::CmpLess => {
                    let right = self.value_stack.pop().unwrap();
                    let left = self.value_stack.pop().unwrap();
                    self.stack_push_val(VmValue::Bool(left.partial_cmp(&right) == Some(std::cmp::Ordering::Less)));
                }
                OpCode::CmpGreater => {
                    let right = self.value_stack.pop().unwrap();
                    let left = self.value_stack.pop().unwrap();
                    self.stack_push_val(VmValue::Bool(left.partial_cmp(&right) == Some(std::cmp::Ordering::Greater)));
                }

                OpCode::NumAdd { num_mode } => infix_num_op!(self, num_mode, +),
                OpCode::NumSubtract { num_mode } => infix_num_op!(self, num_mode, -),
                OpCode::NumMultiply { num_mode } => infix_num_op!(self, num_mode, *),
                OpCode::NumDivide { num_mode } => infix_num_op!(self, num_mode, /),
                OpCode::NumModulo { num_mode } => infix_num_op!(self, num_mode, %),
                OpCode::NumNegate { num_mode } => prefix_num_op!(self, num_mode, -),

                OpCode::BoolNegate => prefix_op!(self, VmValue::Bool(b) => VmValue::Bool(!b)),


                // Tuples!
                OpCode::TupCreate { length } => {
                    let cut_off_index = self.value_stack.len() - length;
                    let elems = self.value_stack.drain(cut_off_index..).collect();
                    self.stack_push_val(VmValue::Tup(elems));
                }
                OpCode::TupArrCreate { length } => {
                    let tup_elem = self.value_stack.pop().unwrap();
                    let elems = (0..*length).map(|_| tup_elem.clone()).collect();
                    self.stack_push_val(VmValue::Tup(elems));
                }

                OpCode::TupPointerGet { index } => {
                    let VmValue::ValuePointer(tup_pointer) = self.value_stack.pop().unwrap() else { unreachable!() };
                    let VmValue::Tup(tup) = (unsafe { &mut *tup_pointer }) else { unreachable!() };

                    let pointer = &raw mut tup[*index];
                    self.stack_push_val(VmValue::ValuePointer(pointer));
                }
                OpCode::TupGet { index } => {
                    let VmValue::Tup(tup) = self.value_stack.pop().unwrap() else { unreachable!() };
                    let val = tup[*index].clone();
                    self.stack_push_val(val);
                }
                OpCode::TupPointerIndex => {
                    let VmValue::Int(i) = self.value_stack.pop().unwrap() else { unreachable!() };

                    let VmValue::ValuePointer(arr_pointer) = self.value_stack.pop().unwrap() else { unreachable!() };
                    let VmValue::Tup(tup) = (unsafe { &mut *arr_pointer }) else { unreachable!() };

                    let i_usize = usize::try_from(i).unwrap();
                    if i < 0 || i_usize >= tup.len() {
                        return Err(ErrType::RuntimeError { msg: format!("Index {i_usize} is out of bounds for arr of length {}.", tup.len()) });
                    }

                    self.stack_push_val(VmValue::ValuePointer(
                        &raw mut tup[i_usize]
                    ));
                }

                OpCode::TupUnpack { length } => {
                    let VmValue::Tup(tup) = self.value_stack.pop().unwrap() else { unreachable!("last value was not a tuple") };
                    assert_eq!(*length, tup.len());

                    self.stack_extend_from_slice(&tup);
                }


                // Strings
                OpCode::StrAdd => infix_op!(self, VmValue::Str(l), VmValue::Str(r) => VmValue::Str(l + &r)),
                OpCode::StrTemplate { length } => {
                    let cut_off_index = self.value_stack.len() - length;
                    let mut string = String::new();
                    for val in self.value_stack.drain(cut_off_index..) {
                        string += &Self::val_to_string(&val);
                    }
                    self.stack_push_val(VmValue::Str(string));
                }
                OpCode::StrTrimPrefix { const_str } => {
                    let VmValue::Str(prefix_str) = chunk.constants[*const_str].clone() else { unreachable!() };
                    let VmValue::Str(target_str) = self.value_stack.pop().unwrap() else { unreachable!() };

                    if target_str.starts_with(&prefix_str) {
                        let remaining = target_str[prefix_str.len()..].to_string();
                        self.value_stack.push(VmValue::Str(remaining));
                        self.value_stack.push(VmValue::Bool(true));
                    } else {
                        self.value_stack.push(VmValue::Str(target_str));
                        self.value_stack.push(VmValue::Bool(false));
                    }
                }
                OpCode::StrTrimUntil { const_str } => {
                    let VmValue::Str(delim_str) = chunk.constants[*const_str].clone() else { unreachable!() };
                    let VmValue::Str(target_str) = self.value_stack.pop().unwrap() else { unreachable!() };

                    if let Some(i) = target_str.find(&delim_str) {
                        let hole = target_str[..i].to_string();
                        let remaining = target_str[i + delim_str.len()..].to_string();

                        self.value_stack.push(VmValue::Str(remaining));
                        self.value_stack.push(VmValue::Str(hole));
                        self.value_stack.push(VmValue::Bool(true));
                    } else {
                        self.value_stack.push(VmValue::Str(target_str.clone()));
                        self.value_stack.push(VmValue::Str(target_str));
                        self.value_stack.push(VmValue::Bool(false));
                    }
                }
                OpCode::StrTrimSuffix { const_str } => {
                    let VmValue::Str(suffix_str) = chunk.constants[*const_str].clone() else { unreachable!() };
                    let VmValue::Str(target_str) = self.value_stack.pop().unwrap() else { unreachable!() };

                    if target_str.ends_with(&suffix_str) {
                        let hole = target_str[..target_str.len() - suffix_str.len()].to_string();
                        self.value_stack.push(VmValue::Str(hole));
                        self.value_stack.push(VmValue::Bool(true));
                    } else {
                        self.value_stack.push(VmValue::Str(target_str.clone()));
                        self.value_stack.push(VmValue::Bool(false));
                    }
                }


                // Control Flow
                OpCode::Jump { offset } => {
                    frame.ip = frame.ip.checked_add_signed(*offset).expect("Instruction pointer jumped out of bounds...");
                }
                OpCode::JumpIfFalse { offset } => {
                    let VmValue::Bool(bool) = self.value_stack.pop().unwrap() else { unreachable!("Expected a boolean value...") };
                    if !bool {
                        frame.ip = frame.ip.checked_add_signed(*offset).expect("Instruction pointer jumped out of bounds...");
                    }
                }

                OpCode::CallFn { arg_count } => {
                    let callee = self.value_stack.pop().unwrap();

                    let first_arg_index = self.value_stack.len() - arg_count;
                    let args = self.value_stack.drain(first_arg_index..).collect::<Vec<_>>();

                    match callee {
                        VmValue::NativeFn(native_fn) => {
                            self.stack_push_val(
                                native_fn(&args)?
                            );
                        }

                        VmValue::Fn { slot } => {
                            self.load_frame_from_index(slot);
                            self.value_stack.extend(args);
                        }

                        _ => unreachable!("tried to call {callee}...")
                    }
                }

                // meta type stuff
                OpCode::MakeTypeRef { mutable } => {
                    let VmValue::Type(a) = self.value_stack.pop().unwrap() else {
                        unreachable!("last value was not a meta type")
                    };
                    let pointer_type = self.type_arena.as_mut().expect("tried to access the type_arena, but there was none...")
                        .add_type(Type::Borrow { inner: a, mutable: *mutable, borrows_var: None });
                    self.stack_push_val(VmValue::Type(pointer_type));
                }

                OpCode::TypeTupToTupType { labels } => {
                    let VmValue::Tup(tup) = self.value_stack.pop().unwrap() else {
                        unreachable!("last value was not a tup")
                    };

                    assert_eq!(tup.len(), labels.len());
                    let tup_type = tup.iter().zip(labels)
                        .map(|(val, label)| {
                            let VmValue::Type(id) = val else {
                                unreachable!("should be a type...")
                            };
                            TypeTuple { label: label.clone(), typ: *id }
                        })
                        .collect();

                    let tup_type = self.type_arena.as_mut().expect("tried to access the type_arena, but there was none...")
                        .add_type(Type::Tup(tup_type));
                    self.stack_push_val(VmValue::Type(tup_type));
                }

                // End of function / program
                OpCode::Return => {
                    let return_value = self.value_stack.pop().unwrap();
                    let new_len = self.value_stack.len() - chunk.local_slots_needed;
                    self.value_stack.truncate(new_len);
                    self.stack_push_val(return_value);
                    self.frames.pop();
                }
                OpCode::Panic => {
                    let VmValue::Str(msg) = self.value_stack.pop().unwrap()
                    else { unreachable!("last value was not a str") };
                    return Err(ErrType::RuntimeError { msg })
                }

                // do literally nothing
                OpCode::NoOp => { }
            }

            if print_debug_execution {
                println!("-> {}", slice_to_string(&self.value_stack, ", "));
            }
        }
    }



    fn stack_push_val(&mut self, val: VmValue) {
        let cap_before_push = self.value_stack.capacity();
        self.value_stack.push(val);
        assert!(cap_before_push <= self.value_stack.capacity(), "Stack Overflow limit of {cap_before_push} reached!");
    }
    fn stack_extend_from_slice(&mut self, slice: &[VmValue]) {
        let cap_before_push = self.value_stack.capacity();
        self.value_stack.extend_from_slice(slice);
        assert!(cap_before_push <= self.value_stack.capacity(), "Stack Overflow limit of {cap_before_push} reached!");
    }



    fn val_to_string(val: &VmValue) -> String {
        match val {
            VmValue::Str(str) => str.clone(),
            VmValue::Int(_)
            | VmValue::Float(_)
            | VmValue::Bool(_)
            | VmValue::Void
            | VmValue::Tup(_) => format!("{val}"),

            _ => panic!("Literal {val:?} cannot be converted into a string.")
        }
    }
}