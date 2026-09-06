use std::fmt::{self, Display, Write};
use crate::{ProgramError, ProgramSourceData, parsing::ast::{AstArena, AstClosure, Expr, ExprId, Pattern, PatternId}};




#[must_use]
pub fn format_program_error<T: Display>(err: &ProgramError<T>, source_data: &ProgramSourceData) -> String {
    // get the ErrType message
    let mut output_str = format!("{}\n{}\n", err.err_type, err.compiler_location);

    if err.span.length > usize::MAX / 2 { return format!("{output_str}(couldn't print where...)") }

    let err_start = err.span.byte_offset;
    let err_end = err.span.byte_offset + err.span.length;
    let mut prefix = "  ";
    let line_number_width = source_data.line_lookup.len().to_string().len();

    let err_start_line = source_data.line_lookup.iter()
        .position(|&s| s > err_start)
        .unwrap_or(source_data.line_lookup.len());

    for line_index in err_start_line.. {
        if line_index > source_data.line_lookup.len() {
            break
        }

        let line_start = source_data.line_lookup[line_index - 1];
        let line_end = if line_index < source_data.line_lookup.len() {
            source_data.line_lookup[line_index]
        } else { source_data.source_code.len() };

        let mut add_line = |line_number: bool, msg: &str, pfx: &str| {
            writeln!(output_str, "{:>width$} |{pfx}{msg}",
                if line_number { line_index.to_string() } else { String::new() },
                width = line_number_width
            ).unwrap();
        };

        // excluding spaces, tabs, \n or \r
        let trimmed_text = source_data.source_code[line_start..line_end].trim_end();
        add_line(true, trimmed_text, prefix);

        let err_starts_before_this_line = err_start < line_start;
        let err_ends_after_this_line = err_end > line_end;

        let hl_end = err_end.min(line_start + trimmed_text.len());

        // underlining logic
        // ^^^^^^^^^^^
        match (err_starts_before_this_line, err_ends_after_this_line) {
            // single-line error, easiest case
            //             ^^^^^
            (false, false) => {
                add_line(false, &format!("{}{}", " ".repeat(err_start - line_start), "^".repeat(hl_end - err_start)), prefix);
                break;
            }
            // multi line errors
            (true, false) => {
                add_line(false, &"^".repeat(hl_end - line_start), "|_");
                break;
            }
            (false, true) => {
                add_line(false, &format!("{}{}", "_".repeat(err_start - line_start), "^".repeat(line_end - err_start)), " _");
                prefix = "| ";
            }
            (true, true) => { },
        }
    }

    output_str
}




impl AstArena {
    #[must_use]
    pub fn display_expr(&self, expr: ExprId) -> String {
        let mut s = String::new();
        self.format_expr_recursive(expr, &mut s, 0, "", true).unwrap();
        s
    }

    fn format_expr_recursive(&self, id: ExprId, s: &mut String, ind: usize, prefix: &str, is_last: bool) -> fmt::Result {
        let expr = self.get_expr(id);

        // Strum gives us the variant name as a static string!
        let variant_name: &str = expr.into();

        write!(s, "{}{} {}{}",
            "  ".repeat(ind),
            if ind == 0 { "" } else if is_last { "└─" } else { "├─" },
            if prefix.is_empty() { String::new() } else { format!("{prefix}: ") },
            variant_name,
        )?;

        match expr {
            Expr::Literal { val } => writeln!(s, " - {val:?}"),
            Expr::IdentifierRef { name, mutable } => writeln!(s, " - {}\"{name}\"", if *mutable {"mut "} else {""}),

            Expr::Prefix { op, right, .. } => {
                writeln!(s, " - {op}")?;
                self.format_expr_recursive(*right, s, ind + 1, "right", true)
            }
            Expr::Infix { op, left, right, .. } => {
                writeln!(s, " - {op}")?;
                self.format_expr_recursive(*left, s, ind + 1, "left", false)?;
                self.format_expr_recursive(*right, s, ind + 1, "right", true)
            }
            Expr::Block { exprs, label } => {
                writeln!(s, " - #{}", label.as_deref().unwrap_or("none"))?;
                for (i, &e) in exprs.iter().enumerate() {
                    self.format_expr_recursive(e, s, ind + 1, "", i == exprs.len() - 1)?;
                }
                Ok(())
            }
            Expr::Assign { pattern, value, extra_op, op_span: _ } => {
                writeln!(s, " - extra_op: {extra_op:?}")?;
                self.format_pattern_recursive(*pattern, s, ind + 1, "pattern", false)?;
                self.format_expr_recursive(*value, s, ind + 1, "value", true)?;
                Ok(())
            }
            Expr::EmptyLet { pattern } => {
                writeln!(s)?;
                self.format_pattern_recursive(*pattern, s, ind + 1, "pattern", true)
            }
            Expr::Const { pattern, value } => {
                writeln!(s)?;
                self.format_pattern_recursive(*pattern, s, ind + 1, "pattern", false)?;
                self.format_expr_recursive(*value, s, ind + 1, "value", true)
            }
            Expr::CustomType { name, value } => {
                writeln!(s, " - name: {name}")?;
                self.format_expr_recursive(*value, s, ind + 1, "value", true)
            }
            Expr::Move { expr } => {
                writeln!(s)?;
                self.format_expr_recursive(*expr, s, ind + 1, "expr", true)
            }
            Expr::Borrow { expr, mutable } => {
                writeln!(s, "mutable: {mutable}")?;
                self.format_expr_recursive(*expr, s, ind + 1, "expr", true)
            }
            Expr::MemberAccess { left, member } | Expr::TypeMemberAccess { left, member } => {
                writeln!(s, " - .{member}")?;
                self.format_expr_recursive(*left, s, ind + 1, "object", true)
            }
            Expr::TemplateString { elems } => {
                writeln!(s)?;
                for (i, &e) in elems.iter().enumerate() {
                    self.format_expr_recursive(e, s, ind + 1, "", i == elems.len() - 1)?;
                }
                Ok(())
            }
            Expr::Tuple { elems } => {
                writeln!(s)?;
                for (i, el) in elems.iter().enumerate() {
                    self.format_expr_recursive(el.expr, s, ind + 1, &format!(".{}", el.label), i == elems.len() - 1)?;
                }
                Ok(())
            }
            Expr::TupleArr { elem, length } => {
                writeln!(s)?;
                self.format_expr_recursive(*elem, s, ind + 1, "elem", false)?;
                self.format_expr_recursive(*length, s, ind + 1, "length", true)
            }
            Expr::Index { left, index } => {
                writeln!(s)?;
                self.format_expr_recursive(*left, s, ind + 1, "arr", false)?;
                self.format_expr_recursive(*index, s, ind + 1, "idx", true)
            }
            Expr::If { condition, then, alt, never_alt: _ } => {
                writeln!(s)?;
                self.format_expr_recursive(*condition, s, ind + 1, "cond", false)?;
                self.format_expr_recursive(*then, s, ind + 1, "then", false)?;
                self.format_expr_recursive(*alt, s, ind + 1, "else", true)
            }
            Expr::Is { value, pattern } => {
                writeln!(s)?;
                self.format_expr_recursive(*value, s, ind + 1, "value:", false)?;
                self.format_pattern_recursive(*pattern, s, ind + 1, "pattern:", true)
            }
            Expr::Match { match_value, arms } => {
                writeln!(s)?;
                self.format_expr_recursive(*match_value, s, ind + 1, "match value", false)?;
                for (i, arm) in arms.iter().enumerate() {
                    let is_last_arm = i == arms.len() - 1;
                    self.format_pattern_recursive(arm.pattern, s, ind + 1, "arm pattern", false)?;
                    self.format_expr_recursive(arm.body, s, ind + 1, "arm body", is_last_arm)?;
                }
                Ok(())
            }
            Expr::While { condition, body, label } => {
                writeln!(s, " - #{label}")?;
                self.format_expr_recursive(*condition, s, ind + 1, "cond", false)?;
                self.format_expr_recursive(*body, s, ind + 1, "body", true)
            }
            Expr::For { pattern, iter_expr, body, label } => {
                writeln!(s, " - #{label}")?;
                self.format_pattern_recursive(*pattern, s, ind + 1, "pattern", false)?;
                self.format_expr_recursive(*iter_expr, s, ind + 1, "iter_expr", false)?;
                self.format_expr_recursive(*body, s, ind + 1, "body", true)
            }
            Expr::Loop { body, label } => {
                writeln!(s, " - #{label}")?;
                self.format_expr_recursive(*body, s, ind + 1, "body", true)
            }
            Expr::FnDefinition { name, closure } => {
                writeln!(s, " - {name}")?;
                self.format_closure_recursive(closure, s, ind + 1, true)
            }
            Expr::Closure { closure, requires_type_annotation } => {
                writeln!(s, " - requires_type_annotation: {requires_type_annotation}")?;
                self.format_closure_recursive(closure, s, ind + 1, true)
            }
            Expr::Call { callee, arguments } => {
                writeln!(s)?;
                self.format_expr_recursive(*callee, s, ind + 1, "func", false)?;
                for (i, &arg) in arguments.iter().enumerate() {
                    self.format_expr_recursive(arg, s, ind + 1, "arg", i == arguments.len() - 1)?;
                }
                Ok(())
            }
            Expr::TypeInstantiation { typ, data } => {
                writeln!(s)?;
                self.format_expr_recursive(*typ, s, ind + 1, "typ", false)?;
                self.format_expr_recursive(*data, s, ind + 1, "typ", true)
            }
            Expr::EnumDefinition { variants } => {
                writeln!(s)?;
                for (i, v) in variants.iter().enumerate() {
                    writeln!(s, "{}  {} {}::{}",
                        "  ".repeat(ind),
                        if i == variants.len() - 1 { "└─" } else { "├─" },
                        if prefix.is_empty() { "" } else { prefix },
                        v.variant_name
                    )?;
                    if let Some(data) = v.attached_tuple {
                        self.format_expr_recursive(data, s, ind + 2, "data", true)?;
                    }
                }
                Ok(())
            }
            Expr::EnumVariant { data } => {
                writeln!(s, " - .{}", data.variant_name)?;
                if let Some(data) = data.attached_tuple {
                    self.format_expr_recursive(data, s, ind + 1, "data", true)?;
                }
                Ok(())
            }
            Expr::ImplBlock { typ, const_exprs } => {
                writeln!(s)?;
                self.format_expr_recursive(*typ, s, ind + 1, "typ", false)?;
                for (i, &e) in const_exprs.iter().enumerate() {
                    self.format_expr_recursive(e, s, ind + 1, "", i == const_exprs.len() - 1)?;
                }
                Ok(())
            }
            Expr::ImplSelf => {
                writeln!(s)
            }
            Expr::Return { expr } => {
                writeln!(s)?;
                self.format_expr_recursive(*expr, s, ind + 1, "val", true)
            }
            Expr::Break { label, expr } => {
                writeln!(s, " - #{}", label.as_deref().unwrap_or("none"))?;
                self.format_expr_recursive(*expr, s, ind + 1, "val", true)
            }
            Expr::Continue { label } => {
                writeln!(s, " - #{}", label.as_deref().unwrap_or("none"))
            }
            Expr::Void | Expr::ParserError => writeln!(s),
        }
    }

    // helper for closures and fndefs
    fn format_closure_recursive(&self, closure: &AstClosure, f: &mut String, ind: usize, is_last: bool) -> fmt::Result {
        for &p in &closure.params {
            self.format_pattern_recursive(p, f, ind, "param", false)?;
        }
        if let Some(ret) = closure.return_type {
            self.format_expr_recursive(ret, f, ind, "return type", false)?;
        }
        self.format_expr_recursive(closure.body, f, ind, "body", is_last)
    }

    // --- PATTERN FORMATTING ---
    // patterns contain Expressions (`Expr` and `Typed`), so they must recursively print too!
    fn format_pattern_recursive(&self, id: PatternId, s: &mut String, ind: usize, prefix: &str, is_last: bool) -> fmt::Result {
        let pat = self.get_pattern(id);
        let variant_name: &str = pat.into();

        write!(s, "{}{} {}{}",
            "  ".repeat(ind),
            if ind == 0 { "" } else if is_last { "└─" } else { "├─" },
            if prefix.is_empty() { String::new() } else { format!("{prefix}: ") },
            variant_name,
        )?;

        match pat {
            Pattern::Wildcard => writeln!(s, " - _"),
            Pattern::Not(p) => {
                writeln!(s)?;
                self.format_pattern_recursive(*p, s, ind + 1, "not", true)
            }

            Pattern::Binding { name, mutable } => writeln!(s, " - {}\"{name}\"", if *mutable {"mut "} else {""}),

            Pattern::Or(patterns) => {
                writeln!(s)?;
                for (i, &p) in patterns.iter().enumerate() {
                    self.format_pattern_recursive(p, s, ind + 1, "", i == patterns.len() - 1)?;
                }
                Ok(())
            }
            Pattern::Tuple(elems) => {
                writeln!(s)?;
                for (i, el) in elems.iter().enumerate() {
                    self.format_pattern_recursive(el.pattern, s, ind + 1, &format!(".{}", el.label), i == elems.len() - 1)?;
                }
                Ok(())
            }
            Pattern::String { before, hole_parts } => {
                writeln!(s, " - \"{before}\"")?;
                for (i, hole_part) in hole_parts.iter().enumerate() {
                    self.format_pattern_recursive(hole_part.0, s, ind + 1, "hole", i == hole_parts.len() - 1)?;
                    write!(s, "\"{}\"", hole_part.1)?;
                }
                Ok(())
            }
            Pattern::TypeDestructor { typ, data } => {
                self.format_expr_recursive(*typ, s, ind + 1, "typ", false)?;
                self.format_pattern_recursive(*data, s, ind + 1, "data", true)
            }
            Pattern::EnumVariant { name, attached_tuple } => {
                writeln!(s, " - {name}")?;
                if let Some(tup) = attached_tuple {
                    self.format_pattern_recursive(*tup, s, ind + 1, "data", true)?;
                }
                Ok(())
            }
            Pattern::Conditional { pattern, cond } => {
                writeln!(s)?;
                self.format_pattern_recursive(*pattern, s, ind + 1, "pattern", false)?;
                self.format_expr_recursive(*cond, s, ind + 1, "cond", true)
            }
            Pattern::Typed { pattern, typ } => {
                writeln!(s)?;
                self.format_pattern_recursive(*pattern, s, ind + 1, "pattern", false)?;
                self.format_expr_recursive(*typ, s, ind + 1, "type", true)
            }
            Pattern::CompareExpr(expr) => {
                writeln!(s)?;
                self.format_expr_recursive(*expr, s, ind + 1, "expr", true)
            }
            Pattern::PlacePointer(expr) => {
                writeln!(s)?;
                self.format_expr_recursive(*expr, s, ind + 1, "placepointer", true)
            }
        }
    }
}





pub fn slice_to_string<T: fmt::Display>(vec: &[T], sep: &str) -> String {
    if let Some((f, other)) = vec.split_first() {
        let mut s = format!("{f}");
        for o in other {
            write!(s, "{sep}{o}").unwrap();
        }
        s
    } else {
        String::new()
    }
    // vec.iter().map(|x| x.to_string()).collect::<Vec<String>>().join(sep)
}
pub fn slice_to_debug_string<T: fmt::Debug>(vec: &[T], sep: &str) -> String {
    if let Some((f, other)) = vec.split_first() {
        let mut s = format!("{f:?}");
        for o in other {
            write!(s, "{sep}{o:?}").unwrap();
        }
        s
    } else {
        String::new()
    }
    // vec.iter().map(|x| format!("{x:?}")).collect::<Vec<String>>().join(sep)
}

#[must_use]
pub fn slice_to_or_string(vec: &[String], sep: &str) -> String {
    match vec {
        [] => String::new(),
        [only] => only.clone(),
        [before @ .., last] => format!("{} {sep} {last}", before.join(", "))
    }
}