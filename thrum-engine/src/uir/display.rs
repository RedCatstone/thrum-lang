use std::fmt::{self, Display, Formatter};

use crate::uir::ir::{
    AggregateKind, Constant, Operand, Place, Projection, RValue, Statement,
    Terminator, UirFunction,
};

impl Display for UirFunction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        writeln!(f, "fn {}({}) -> {:?} {{", self.name, self.format_params(), self.return_type)?;

        // Print locals
        for local in &self.locals {
            write!(f, "  // _{}: {:?}", local.id.0, local.type_id)?;
            if let Some(debug) = &local.debug_name {
                write!(f, " [name: \"{debug}\"]")?;
            }
            if local.is_mut {
                write!(f, " [mut]")?;
            }
            writeln!(f)?;
        }
        writeln!(f)?;

        // Print basic blocks
        for (i, block) in self.blocks.iter().enumerate() {
            writeln!(f, "  block{i}: {{")?;
            for stmt in &block.statements {
                writeln!(f, "    {stmt}")?;
            }
            writeln!(f, "    {}", block.terminator)?;
            writeln!(f, "  }}")?;
        }
        writeln!(f, "}}")
    }
}

impl UirFunction {
    fn format_params(&self) -> String {
        let params: Vec<String> = (1..=self.param_count)
            .map(|i| {
                let local = &self.locals[i];
                format!("_{}: {:?}", i, local.type_id)
            })
            .collect();
        params.join(", ")
    }
}

impl Display for Statement {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assign(place, rvalue) => write!(f, "{place} = {rvalue};"),
            Self::SetDiscriminant { place, variant_index } => {
                write!(f, "discriminant({place}) = {variant_index};")
            }
            Self::StorageLive(local) => write!(f, "StorageLive(_{});", local.0),
            Self::StorageDead(local) => write!(f, "StorageDead(_{});", local.0),
            Self::Nop => write!(f, "nop;"),
        }
    }
}

impl Display for Terminator {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Goto(target) => write!(f, "goto -> bb{};", target.0),
            Self::MatchOnInt { discr, targets, otherwise } => {
                write!(f, "switchInt({discr}) -> [")?;
                for (val, target) in targets {
                    write!(f, "{val}: bb{}, ", target.0)?;
                }
                write!(f, "otherwise: bb{}];", otherwise.0)
            }
            Self::Call { func, args, destination, target } => {
                let args_str: Vec<String> = args.iter().map(std::string::ToString::to_string).collect();
                write!(f, "{destination} = call {func}({})", args_str.join(", "))?;
                match target {
                    Some(target_bb) => write!(f, " -> [return: bb{}];", target_bb.0),
                    None => write!(f, " -> [diverges];"),
                }
            }
            Self::Return => write!(f, "return;"),
            Self::Panic { msg } => write!(f, "panic({msg});"),
            Self::Unreachable => write!(f, "unreachable;"),
        }
    }
}

impl Display for Place {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut s = format!("_{}", self.local.0);
        for proj in &self.projections {
            s = match proj {
                Projection::Deref => format!("(*{s})"),
                Projection::Field(idx) => format!("{s}.{idx}"),
                Projection::Index(idx) => format!("{s}[_{}]", idx.0),
                Projection::ConstantIndex(idx) => format!("{s}[{idx}]"),
                Projection::Downcast(idx) => format!("({s} as variant#{idx})"),
            }
        }
        write!(f, "{s}")
    }
}

impl Display for Operand {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Copy(place) => write!(f, "Copy({place})"),
            Self::Move(place) => write!(f, "Move({place})"),
            Self::Constant(c) => write!(f, "{c}"),
        }
    }
}

impl Display for Constant {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{i}_i64"),
            Self::Float(fl) => write!(f, "{fl}_f64"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Str(s) => write!(f, "\"{s}\""),
            Self::Void => write!(f, "()"),
            Self::Type(t) => write!(f, "type#{:?}", t.0),
            Self::Function(func_id) => write!(f, "fn#{}", func_id.0),
            Self::NativeFn(name) => write!(f, "native_fn({name})"),
        }
    }
}

impl Display for RValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use(op) => write!(f, "{op}"),
            Self::UnaryOp(op, num_mode, operand) => write!(f, "{op:?}{num_mode:?}({operand})"),
            Self::BinaryOp(op, num_mode, left, right) => write!(f, "{op:?}{num_mode:?}({left}, {right})"),
            Self::MakeRef { mutable, place } => {
                write!(f, "&{}{place}", if *mutable { "mut " } else { "" })
            }
            Self::UseDiscriminant(place) => write!(f, "discriminant({place})"),
            Self::MakeAggregate(kind, operands) => {
                let ops: Vec<String> = operands.iter().map(std::string::ToString::to_string).collect();
                match kind {
                    AggregateKind::Tuple => write!(f, "Tuple({})", ops.join(", ")),
                    AggregateKind::Array => write!(f, "Array([{}])", ops.join(", ")),
                    AggregateKind::Enum { variant_index, .. } => {
                        write!(f, "EnumVariant(#{variant_index}, {})", ops.join(", "))
                    }
                    AggregateKind::FormatString { str_pieces, .. } => {
                        write!(f, "FormatString(pieces: {:?}, holes: [{}])", str_pieces, ops.join(", "))
                    }
                }
            }
            Self::MakeSlice { ptr, len } => write!(f, "Slice(ptr: {ptr}, len: {len})"),
            Self::GetLen(place) => write!(f, "Len({place})"),
            Self::Cast { operand, target_type } => write!(f, "Cast({operand} as {target_type:?})"),
        }
    }
}