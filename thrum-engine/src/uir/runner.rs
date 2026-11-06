use simply_colored::{RESET, YELLOW};
use crate::{
    ErrType, parsing::ast::{BinOp, UnaryOp}, typing::{TupMode, Type, TypeArena, TypeId}, uir::ir::{
        BlockId, Constant, FunctionId, LocalId, Operand, RValue, Statement, Terminator, UirFunction,
    }, vm_compiling::{NumMode, VmValue},
};

pub struct UirRunner<'a> {
    function: &'a UirFunction,
    memory: Vec<u8>,
    frames: Vec<Frame>,
}

struct Frame {
    function_id: FunctionId,
    current_block: BlockId,
    statement_idx: usize,
    base_offset: usize, // where this frame's locals start
}

impl<'a> UirRunner<'a> {
    pub fn start(function: &'a UirFunction, type_arena: &TypeArena) -> Result<VmValue, ErrType> {
        let mut runner = Self::new(function);
        runner.run()?;

        // Return value is stored in `_0`
        Ok(runner.read_vm_value(0, function.return_type, type_arena))
    }

    fn new(function: &'a UirFunction) -> Self {
        let mut runner = Self {
            function,
            memory: Vec::new(),
            frames: Vec::new(),
        };
        runner.push_frame(function.id);
        runner
    }

    fn push_frame(&mut self, function_id: FunctionId) {
        let base_offset = self.memory.len();
        self.memory.resize(base_offset + self.function.frame_size, 0);

        self.frames.push(Frame {
            function_id,
            current_block: BlockId(0),
            statement_idx: 0,
            base_offset,
        });
    }

    fn run(&mut self) -> Result<(), ErrType> {
        while let Some(frame) = self.frames.last_mut() {
            let block = &self.function.blocks[frame.current_block.0 as usize];

            if let Some(stmt) = block.statements.get(frame.statement_idx) {
                frame.statement_idx += 1;
                self.step_statement(stmt)?;

                print!("{:<30} [", format!("{stmt}"));
                for i in 0..self.memory.len() / 8 {
                    print!("{}", self.read_int(i * 8));
                    if i != self.memory.len() / 8 - 1 { print!(" ") }
                }
                println!("]");
            }
            else {
                let finished = self.step_terminator(&block.terminator)?;
                println!("{:<30}", format!("{}", block.terminator));
                if finished {
                    println!();
                    break
                }
                println!("{YELLOW}--> {}{RESET}", self.function.format_block_name(self.frames.last().unwrap().current_block.0 as usize));
            }
        }
        Ok(())
    }

    fn step_statement(&mut self, stmt: &Statement) -> Result<(), ErrType> {
        match stmt {
            Statement::Assign(dest_local, rvalue) => {
                let dest_offset = self.resolve_local_offset(*dest_local);

                self.evaluate_rvalue(rvalue, dest_offset, *dest_local)?;
            }
            Statement::Store { pointer, value } => {
                let target_addr = self.read_pointer(self.resolve_local_offset(*pointer));
                self.copy_operand(value, target_addr);
            }
            Statement::SetDiscriminant { local, variant_index } => {
                let offset = self.resolve_local_offset(*local);
                self.write_int(offset, (*variant_index).try_into().unwrap());
            }
            Statement::Nop => {}
            _ => todo!("Statement {stmt:?} not implemented yet"),
        }
        Ok(())
    }

    fn evaluate_rvalue(&mut self, rvalue: &RValue, dest: usize, dest_local: LocalId) -> Result<(), ErrType> {
        match rvalue {
            RValue::Use(op) => {
                self.copy_operand(op, dest);
            }
            RValue::BinaryOp(op, num_mode, left, right) => {
                match num_mode {
                    NumMode::Int => {
                        let a = self.eval_operand_int(left);
                        let b = self.eval_operand_int(right);
                        match op {
                            BinOp::Add => self.write_int(dest, a + b),
                            BinOp::Sub => self.write_int(dest, a - b),
                            BinOp::Mul => self.write_int(dest, a * b),
                            BinOp::Div => {
                                if b == 0 { return Err(ErrType::RuntimeError { msg: "division by zero".into() }) }
                                self.write_int(dest, a.wrapping_div(b));
                            }
                            BinOp::Rem => {
                                if b == 0 { return Err(ErrType::RuntimeError { msg: "division by zero".into() }) }
                                self.write_int(dest, a.wrapping_rem(b));
                            }
                            BinOp::Equal => self.write_int(dest, i64::from(a == b)),
                            BinOp::NotEqual => self.write_int(dest, i64::from(a != b)),
                            BinOp::Less => self.write_int(dest, i64::from(a < b)),
                            BinOp::LessEqual => self.write_int(dest, i64::from(a <= b)),
                            BinOp::Greater => self.write_int(dest, i64::from(a > b)),
                            BinOp::GreaterEqual => self.write_int(dest, i64::from(a >= b)),
                            _ => todo!("BinOp {op:?} not implemented yet"),
                        }
                    }
                    NumMode::Float => {
                        let a = self.eval_operand_float(left);
                        let b = self.eval_operand_float(right);

                        #[expect(clippy::float_cmp)]
                        match op {
                            BinOp::Add => self.write_float(dest, a + b),
                            BinOp::Sub => self.write_float(dest, a - b),
                            BinOp::Mul => self.write_float(dest, a * b),
                            BinOp::Div => self.write_float(dest, a / b),
                            BinOp::Rem => self.write_float(dest, a % b),
                            BinOp::Equal => self.write_int(dest, i64::from(a == b)),
                            BinOp::NotEqual => self.write_int(dest, i64::from(a != b)),
                            BinOp::Less => self.write_int(dest, i64::from(a < b)),
                            BinOp::LessEqual => self.write_int(dest, i64::from(a <= b)),
                            BinOp::Greater => self.write_int(dest, i64::from(a > b)),
                            BinOp::GreaterEqual => self.write_int(dest, i64::from(a >= b)),
                            _ => todo!("BinOp {op:?} not implemented yet"),
                        }
                    }
                }
            }
            RValue::UnaryOp(UnaryOp::Negative, num_mode, op) => {
                match num_mode {
                    NumMode::Int => self.write_int(dest, -self.eval_operand_int(op)),
                    NumMode::Float => self.write_float(dest, -self.eval_operand_float(op))
                }
            }
            RValue::MakeRef { mutable: _, local } => {
                let offset = self.resolve_local_offset(*local);
                self.write_pointer(dest, offset);
            }
            RValue::Deref(local) => {
                let pointer_offset = self.read_pointer(self.resolve_local_offset(*local));
                let dest_size = self.function.locals[dest_local.0 as usize].size;

                self.memory.copy_within(
                    pointer_offset..pointer_offset + dest_size,
                    dest,
                );
            }


            RValue::MakeTuple(ops) => {
                let mut cur_offset = dest;
                for op in ops {
                    let size = self.operand_size(op);
                    self.copy_operand(op, cur_offset);
                    cur_offset += size;
                }
            }
            RValue::MakeTupleArr { elem, length } => {
                let elem_size = self.operand_size(elem);
                for i in 0..*length {
                    self.copy_operand(elem, dest + i * elem_size);
                }
            }

            RValue::TupleGet { tuple, offset } => {
                let tup_offset = self.resolve_local_offset(*tuple);
                let dest_size = self.function.locals[dest_local.0 as usize].size;

                self.memory.copy_within(
                    tup_offset + offset..tup_offset + offset + dest_size,
                    dest,
                );
            }
            RValue::TupleGetRef { tuple, offset } => {
                let base_ptr = self.read_pointer(self.resolve_local_offset(*tuple));
                self.write_pointer(dest, base_ptr + offset);
            }

            RValue::IndexRef { base, index, elem_size, len } => {
                let base_ptr = self.read_pointer(self.resolve_local_offset(*base));
                let idx = self.eval_operand_int(index);

                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                if idx < 0 || idx as usize >= *len {
                    return Err(ErrType::RuntimeError {
                        msg: format!("Index {idx} is out of bounds for arr of length {len}."),
                    });
                }

                let idx_usize: usize = idx.try_into().unwrap();
                self.write_pointer(dest, base_ptr + idx_usize * elem_size);
            }

            RValue::ReadDiscriminant(local) => {
                let enum_offset = self.resolve_local_offset(*local);
                let discr = self.read_int(enum_offset);
                self.write_int(dest, discr);
            }
            RValue::MakeEnum { enum_id: _, variant_index, payload } => {
                // tag at offset 0
                self.write_int(dest, (*variant_index).try_into().unwrap());
                // payload at offset 8
                self.copy_operand(payload, dest + 8);
            }

            _ => todo!("Rvalue {rvalue:?} not implemented yet"),
        }
        Ok(())
    }

    fn step_terminator(&mut self, term: &Terminator) -> Result<bool, ErrType> {
        match term {
            Terminator::Goto(target) => {
                let frame = self.frames.last_mut().unwrap();
                frame.current_block = *target;
                frame.statement_idx = 0;
                Ok(false)
            }
            Terminator::MatchOnInt { discr, targets, otherwise } => {
                let val = self.eval_operand_int(discr);
                let next_bb = targets
                    .iter()
                    .find(|(target_val, _)| *target_val == val)
                    .map_or(*otherwise, |(_, bb)| *bb);

                let frame = self.frames.last_mut().unwrap();
                frame.current_block = next_bb;
                frame.statement_idx = 0;
                Ok(false)
            }
            Terminator::Return => {
                self.frames.pop();
                Ok(self.frames.is_empty())
            }
            Terminator::Panic { msg } => {
                let val = self.eval_operand_int(msg);
                Err(ErrType::RuntimeError { msg: format!("panic: {val}") })
            }
            Terminator::Unreachable => Err(ErrType::RuntimeError {
                msg: "entered unreachable code...".into(),
            }),
            Terminator::Call { .. } => todo!("Terminator {term:?} not implemented yet"),
        }
    }

    fn resolve_local_offset(&self, local: LocalId) -> usize {
        self.frames.last().unwrap().base_offset + self.function.locals[local.0 as usize].offset
    }


    #[track_caller]
    fn copy_operand(&mut self, op: &Operand, dest_offset: usize) {
        match op {
            Operand::Constant(Constant::Void) => {} // 0 bytes, do nothing TODO: generalize this better so this case doesn't need to be here
            Operand::Constant(c) => {
                let bytes = Self::eval_constant_bytes(c);
                self.write_bytes(dest_offset, &bytes);
            }
            Operand::Copy(src) | Operand::Move(src) => {
                let src_size = self.function.locals[src.0 as usize].size;
                let src_offset = self.resolve_local_offset(*src);
                self.memory
                    .copy_within(src_offset..src_offset + src_size, dest_offset);
            }
        }
    }

    fn operand_size(&self, op: &Operand) -> usize {
        match op {
            Operand::Constant(Constant::Void) => 0,
            Operand::Constant(_) => 8,
            Operand::Copy(l) | Operand::Move(l) => self.function.locals[l.0 as usize].size,
        }
    }

    fn eval_constant_bytes(c: &Constant) -> [u8; 8] {
        match c {
            Constant::Int(i) => i.to_ne_bytes(),
            Constant::Float(f) => f.to_ne_bytes(),
            Constant::Bool(b) => i64::from(*b).to_ne_bytes(),
            Constant::Void => [0; 8],
            _ => [255; 8],
        }
    }

    fn eval_operand_bytes(&self, op: &Operand) -> [u8; 8] {
        match op {
            Operand::Constant(c) => Self::eval_constant_bytes(c),
            Operand::Copy(local) | Operand::Move(local) => {
                let offset = self.resolve_local_offset(*local);
                let mut bytes = [0; 8];
                bytes.copy_from_slice(&self.memory[offset..offset + 8]);
                bytes
            }
        }
    }
    fn eval_operand_int(&self, op: &Operand) -> i64 {
        i64::from_ne_bytes(self.eval_operand_bytes(op))
    }
    fn eval_operand_float(&self, op: &Operand) -> f64 {
        f64::from_ne_bytes(self.eval_operand_bytes(op))
    }

    #[track_caller]
    fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
        self.memory[offset..offset + bytes.len()].copy_from_slice(bytes);
    }
    fn write_pointer(&mut self, offset: usize, val: usize) {
        self.write_bytes(offset, &val.to_ne_bytes());
    }
    fn write_int(&mut self, offset: usize, val: i64) {
        self.write_bytes(offset, &val.to_ne_bytes());
    }
    fn write_float(&mut self, offset: usize, val: f64) {
        self.write_bytes(offset, &val.to_ne_bytes());
    }

    fn read_pointer(&self, offset: usize) -> usize {
        const BYTES: usize = std::mem::size_of::<usize>();
        let mut bytes = [0; BYTES];
        bytes.copy_from_slice(&self.memory[offset..offset + BYTES]);
        usize::from_ne_bytes(bytes)
    }
    fn read_int(&self, offset: usize) -> i64 {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.memory[offset..offset + 8]);
        i64::from_ne_bytes(bytes)
    }
    fn read_float(&self, offset: usize) -> f64 {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.memory[offset..offset + 8]);
        f64::from_ne_bytes(bytes)
    }



    #[must_use] pub fn read_vm_value(&self, offset: usize, type_id: TypeId, type_arena: &TypeArena) -> VmValue {
        match type_arena.get_type(type_id) {
            Type::NumInt => VmValue::Int(self.read_int(offset)),
            Type::NumFloat => VmValue::Float(self.read_float(offset)),
            Type::Bool => VmValue::Bool(self.read_int(offset) != 0),
            Type::Void => VmValue::Void,
            Type::Tup(mode) => {
                let mut vals = Vec::with_capacity(mode.len());
                match mode {
                    TupMode::Tup(elems) => {
                        let mut cur_offset = offset;
                        for elem in elems {
                            let size = type_arena.type_size(elem.typ);
                            vals.push(self.read_vm_value(cur_offset, elem.typ, type_arena));
                            cur_offset += size;
                        }
                    }
                    TupMode::Arr(elem, len) => {
                        let size = type_arena.type_size(elem);
                        for i in 0..len {
                            vals.push(self.read_vm_value(offset + i * size, elem, type_arena));
                        }
                    }
                }
                VmValue::Tup(vals)
            }
            Type::Enum(id) => {
                let tag = self.read_int(offset);
                let tag_usize: usize = tag.try_into().unwrap();

                let variant_type = type_arena.enum_defs[id.0 as usize].variants[tag_usize].1;
                let payload = self.read_vm_value(offset + 8, variant_type, type_arena);
                VmValue::Tup(vec![VmValue::Int(tag), payload])
            }
            Type::CustomType(_, inner)
            | Type::EnumVariant { inner, .. } => self.read_vm_value(offset, inner, type_arena),

            // TODO: this final dereffing should be done in the typechecker instead.
            Type::Borrow { inner, .. } => {
                let ptr = self.read_pointer(offset);
                self.read_vm_value(ptr, inner, type_arena)
            }
            typ => todo!("{typ:?} not yet handled")
        }
    }
}