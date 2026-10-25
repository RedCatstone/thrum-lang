use std::{collections::HashMap, io::Write};

use crate::{ErrType, typing::{Type, TypeArena, TypeId}, vm_compiling::VmValue};

pub struct ThrumValue {
    pub typ: Type,
    pub val: VmValue,
    pub is_prelude: bool,
}

#[derive(Default)]
pub struct ThrumModule {
    pub sub_modules: HashMap<String, Self>,
    pub values: HashMap<String, ThrumValue>,
    pub type_impls: HashMap<TypeId, HashMap<String, ThrumValue>>
}


pub fn get_native_lib(type_arena: &mut TypeArena) -> ThrumModule {
    let mut std_module = ThrumModule::default();

    let str_slice_type = type_arena.get_static_str_borrow();

    let mut io_module = ThrumModule::default();
    io_module.values.insert("print".to_string(), ThrumValue {
        typ: Type::Fn { param_types: vec![TypeId::FMT_STR], return_type: TypeId::VOID },
        val: VmValue::NativeFn(native_print),
        is_prelude: true,
    });
    io_module.values.insert("panic".to_string(), ThrumValue {
        typ: Type::Fn { param_types: vec![str_slice_type], return_type: TypeId::NEVER },
        val: VmValue::NativeFn(native_panic),
        is_prelude: true,
    });
    std_module.sub_modules.insert("io".to_string(), io_module);

    std_module.values.insert("int".to_string(), ThrumValue { typ: Type::MetaType, val: VmValue::Type(TypeId::INT), is_prelude: true });
    std_module.values.insert("float".to_string(), ThrumValue { typ: Type::MetaType, val: VmValue::Type(TypeId::FLOAT), is_prelude: true });
    std_module.values.insert("bool".to_string(), ThrumValue { typ: Type::MetaType, val: VmValue::Type(TypeId::BOOL), is_prelude: true });
    std_module.values.insert("str".to_string(), ThrumValue { typ: Type::MetaType, val: VmValue::Type(TypeId::STR), is_prelude: true });

    let mut str_methods = HashMap::new();
    str_methods.insert("len".to_string(), ThrumValue {
        typ: Type::Fn { param_types: vec![str_slice_type], return_type: TypeId::INT },
        val: VmValue::NativeFn(native_slice_len),
        is_prelude: true
    });
    std_module.type_impls.insert(TypeId::STR, str_methods);

    let mut float_methods = HashMap::new();
    float_methods.insert("NAN".to_string(), ThrumValue { typ: Type::NumFloat, val: VmValue::Float(f64::NAN), is_prelude: true });
    float_methods.insert("INFINITY".to_string(), ThrumValue { typ: Type::NumFloat, val: VmValue::Float(f64::INFINITY), is_prelude: true });
    float_methods.insert("NEG_INFINITY".to_string(), ThrumValue { typ: Type::NumFloat, val: VmValue::Float(f64::NEG_INFINITY), is_prelude: true });
    float_methods.insert("MAX".to_string(), ThrumValue { typ: Type::NumFloat, val: VmValue::Float(f64::MAX), is_prelude: true });
    float_methods.insert("MIN".to_string(), ThrumValue { typ: Type::NumFloat, val: VmValue::Float(f64::MIN), is_prelude: true });
    float_methods.insert("MIN_POSITIVE".to_string(), ThrumValue { typ: Type::NumFloat, val: VmValue::Float(f64::MIN_POSITIVE), is_prelude: true });
    float_methods.insert("MAX_NEGATIVE".to_string(), ThrumValue { typ: Type::NumFloat, val: VmValue::Float(-f64::MIN_POSITIVE), is_prelude: true });
    std_module.type_impls.insert(TypeId::FLOAT, float_methods);

    std_module
}



pub fn native_print(val: &[VmValue]) -> Result<VmValue, ErrType> {
    match &val[0] {
        VmValue::Slice(slice) => println!("{}", slice.as_str()),
        VmValue::FormatString(fmt) => {
            let mut stdout = std::io::stdout().lock();
            fmt.write_to(&mut stdout).map_err(|e| ErrType::RuntimeError { msg: e.to_string() })?;
            let _ = stdout.write_all(b"\n");
        }
        _ => panic!("function called with wrong argument..."),
    }

    Ok(VmValue::Void)
}

pub fn native_panic(val: &[VmValue]) -> Result<VmValue, ErrType> {
    let VmValue::Slice(slice) = &val[0] else { panic!("function called with wrong argument...") };
    println!("{}", slice.as_str());

    Err(ErrType::RuntimeError { msg: slice.as_str().to_string() })
}

pub fn native_slice_len(val: &[VmValue]) -> Result<VmValue, ErrType> {
    let VmValue::Slice(slice) = &val[0] else { panic!("function called with wrong argument...") };
    Ok(VmValue::Int(slice.len.try_into().unwrap()))
}



pub fn display_int(value: &VmValue, out: &mut dyn std::io::Write) {
    let VmValue::ValuePointer(p) = value else { unreachable!() };
    let VmValue::Int(n) = (unsafe { &**p }) else { unreachable!() };
    write!(out, "{n}").unwrap();
}
pub fn display_float(value: &VmValue, out: &mut dyn std::io::Write) {
    let VmValue::ValuePointer(p) = value else { unreachable!() };
    let VmValue::Float(n) = (unsafe { &**p }) else { unreachable!() };
    write!(out, "{n}").unwrap();
}
pub fn display_bool(value: &VmValue, out: &mut dyn std::io::Write) {
    let VmValue::ValuePointer(p) = value else { unreachable!() };
    let VmValue::Bool(n) = (unsafe { &**p }) else { unreachable!() };
    write!(out, "{n}").unwrap();
}
pub fn display_fmt_str(value: &VmValue, out: &mut dyn std::io::Write) {
    let VmValue::ValuePointer(p) = value else { unreachable!() };
    let VmValue::FormatString(n) = (unsafe { &**p }) else { unreachable!() };
    n.write_to(out).unwrap();
}
pub fn display_str(value: &VmValue, out: &mut dyn std::io::Write) {
    let VmValue::Slice(n) = value else { unreachable!() };
    write!(out, "{}", n.as_str()).unwrap();
}