use crate::{
    ErrType,
    uir::ir::{
        BinOp, BlockId, Constant, FunctionId, Operand, Place, Projection, RValue,
        Statement, Terminator, UirFunction, UnaryOp,
    },
};

pub struct UirRunner<'a> {
    function: &'a UirFunction,
    pub memory: Vec<u8>,
    frames: Vec<Frame>,
}

struct Frame {
    function_id: FunctionId,
    current_block: BlockId,
    statement_idx: usize,
    base_offset: usize, // where this frame's locals start
}

impl<'a> UirRunner<'a> {
    pub fn start(function: &'a UirFunction) -> Result<i64, ErrType> {
        let mut runner = Self::new(function);
        runner.run()?;

        // Return value is stored in `_0` (first 8 bytes of the root frame)
        Ok(runner.read_int_at(0))
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
            Statement::Assign(dest_place, rvalue) => {
                let dest_offset = self.resolve_place(dest_place);

                match rvalue {
                    RValue::Use(op) => {
                        let val = self.eval_operand_int(op);
                        self.write_int(dest_offset, val);
                    }
                    RValue::BinaryOp(op, left, right) => {
                        let a = self.eval_operand_int(left);
                        let b = self.eval_operand_int(right);
                        let res = match op {
                            BinOp::Add => a + b,
                            BinOp::Sub => a - b,
                            BinOp::Mul => a * b,
                            BinOp::Div => {
                                match a.checked_div(b) {
                                    Some(res) => res,
                                    None => return Err(ErrType::RuntimeError { msg: "division by zero".into() })
                                }
                            }
                            BinOp::Rem => {
                                match a.checked_rem(b) {
                                    Some(res) => res,
                                    None => return Err(ErrType::RuntimeError { msg: "division by zero".into() })
                                }
                            }
                            BinOp::Eq => i64::from(a == b),
                            BinOp::Ne => i64::from(a != b),
                            BinOp::Lt => i64::from(a < b),
                            BinOp::Le => i64::from(a <= b),
                            BinOp::Gt => i64::from(a > b),
                            BinOp::Ge => i64::from(a >= b),
                            _ => todo!("BinOp {op:?} not implemented yet"),
                        };
                        self.write_int(dest_offset, res);
                    }
                    RValue::UnaryOp(UnaryOp::Neg, op) => {
                        let a = self.eval_operand_int(op);
                        self.write_int(dest_offset, -a);
                    }
                    _ => todo!("Rvalue {rvalue:?} not implemented yet"),
                }
            }
            Statement::Nop => {}
            _ => todo!("Statement {stmt:?} not implemented yet"),
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
            _ => todo!("Terminator {term:?} not implemented yet"),
        }
    }

    fn resolve_place(&self, place: &Place) -> usize {
        let frame = self.frames.last().unwrap();
        let mut offset = frame.base_offset + (place.local.0 as usize) * 8;

        for proj in &place.projections {
            match proj {
                // read pointer value (which is a byte offset into self.memory)
                Projection::Deref => {
                    let ptr_bytes = self.read_int_at(offset);
                    offset = usize::try_from(ptr_bytes).unwrap();
                }
                // field index: +8 bytes per field
                Projection::Field(idx) => {
                    offset += idx * 8;
                }
                // dynamic array index
                Projection::Index(idx_local) => {
                    let idx = self.read_int_at(frame.base_offset + (idx_local.0 as usize) * 8);
                    offset += (usize::try_from(idx).unwrap()) * 8;
                }
                _ => todo!("Projection {proj:?} not supported yet"),
            }
        }
        offset
    }

    fn eval_operand_int(&self, op: &Operand) -> i64 {
        match op {
            Operand::Constant(Constant::Int(i)) => *i,
            Operand::Constant(Constant::Bool(b)) => i64::from(*b),
            Operand::Copy(place) | Operand::Move(place) => {
                let offset = self.resolve_place(place);
                self.read_int_at(offset)
            }
            _ => todo!("Operand {op:?} not supported yet"),
        }
    }

    fn write_int(&mut self, offset: usize, val: i64) {
        let bytes = val.to_ne_bytes();
        self.memory[offset..offset + 8].copy_from_slice(&bytes);
    }

    fn read_int_at(&self, offset: usize) -> i64 {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.memory[offset..offset + 8]);
        i64::from_ne_bytes(bytes)
    }
}