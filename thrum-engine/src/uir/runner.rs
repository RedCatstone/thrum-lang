use crate::{
    ErrType, parsing::ast::{BinOp, UnaryOp}, typing::{Type, TypeArena}, uir::ir::{
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

        // Return value is stored in `_0` (first 8 bytes of the root frame)
        let return_bytes = runner.memory[0..8].try_into().unwrap();
        let return_type = type_arena.get_type(function.return_type);
        let vm_val = VmValue::from_bytes(return_bytes, &return_type);

        Ok(vm_val)
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
        // TODO: make this not a fixed 8 bytes per local
        let frame_size = self.function.locals.len() * 8;
        self.memory.resize(base_offset + frame_size, 0);

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

                println!("{:<30} {:?}", format!("{stmt}"), self.memory);
            }
            else {
                let finished = self.step_terminator(&block.terminator)?;
                if finished {
                    break
                }
            }
        }
        Ok(())
    }

    fn step_statement(&mut self, stmt: &Statement) -> Result<(), ErrType> {
        match stmt {
            Statement::Assign(dest_local, rvalue) => {
                let dest_offset = self.resolve_local_offset(*dest_local);

                self.evaluate_rvalue(rvalue, dest_offset)?;
            }
            Statement::Nop => {}
            _ => todo!("Statement {stmt:?} not implemented yet"),
        }
        Ok(())
    }

    fn evaluate_rvalue(&mut self, rvalue: &RValue, dest: usize) -> Result<(), ErrType> {
        match rvalue {
            RValue::Use(op) => {
                self.write_bytes(dest, &self.eval_operand_bytes(op));
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
                            BinOp::Div => self.write_int(dest, match a.checked_div(b) {
                                Some(res) => res,
                                None => return Err(ErrType::RuntimeError { msg: "division by zero".into() })
                            }),
                            BinOp::Rem => self.write_int(dest, match a.checked_rem(b) {
                                Some(res) => res,
                                None => return Err(ErrType::RuntimeError { msg: "division by zero".into() })
                            }),
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

                let mut bytes = [0; 8];
                bytes.copy_from_slice(&self.memory[pointer_offset..pointer_offset + 8]);

                self.write_bytes(dest, &bytes);
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
        self.frames.last().unwrap().base_offset + (local.0 as usize) * 8
    }


    fn eval_operand_bytes(&self, op: &Operand) -> [u8; 8] {
        match op {
            Operand::Constant(Constant::Int(i)) => i.to_ne_bytes(),
            Operand::Constant(Constant::Float(f)) => f.to_ne_bytes(),
            Operand::Constant(Constant::Bool(b)) => i64::from(*b).to_ne_bytes(),
            Operand::Constant(Constant::Void) => [0; _],
            Operand::Copy(local) | Operand::Move(local) => {
                let offset = self.resolve_local_offset(*local);
                let mut bytes = [0; 8];
                bytes.copy_from_slice(&self.memory[offset..offset + 8]);
                bytes
            }
            Operand::Constant(_) => todo!("Operand {op:?} not supported yet"),
        }
    }
    fn eval_operand_int(&self, op: &Operand) -> i64 {
        i64::from_ne_bytes(self.eval_operand_bytes(op))
    }
    fn eval_operand_float(&self, op: &Operand) -> f64 {
        f64::from_ne_bytes(self.eval_operand_bytes(op))
    }

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
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.memory[offset..offset + 8]);
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
}



impl VmValue {
    #[must_use] pub fn from_bytes(bytes: [u8; 8], typ: &Type) -> Self {
        match typ {
            Type::Bool => Self::Bool(i64::from_ne_bytes(bytes).try_into().unwrap()),
            Type::NumInt => Self::Int(i64::from_ne_bytes(bytes)),
            Type::NumFloat => Self::Float(f64::from_ne_bytes(bytes)),
            Type::Void => Self::Void,
            _ => todo!("{typ:?} not yet handled")
        }
    }
}