use std::collections::HashMap;

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



pub fn get_native_lib(_type_arena: &mut TypeArena) -> ThrumModule {
    let mut std_module = ThrumModule::default();

    let mut io_module = ThrumModule::default();
    io_module.values.insert("print".to_string(), ThrumValue {
        typ: Type::Fn { param_types: vec![TypeId::STR], return_type: TypeId::VOID },
        val: VmValue::NativeFn(native_print),
        is_prelude: true,
    });
    io_module.values.insert("panic".to_string(), ThrumValue {
        typ: Type::Fn { param_types: vec![TypeId::STR], return_type: TypeId::NEVER },
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
        typ: Type::Fn { param_types: vec![TypeId::STR], return_type: TypeId::STR },
        val: VmValue::NativeFn(native_str_len),
        is_prelude: true
    });
    std_module.type_impls.insert(TypeId::STR, str_methods);

    std_module
}



pub fn native_print(val: &[VmValue]) -> Result<VmValue, ErrType> {
    let VmValue::Str(str) = &val[0] else { panic!("function called with wrong argument...") };
    println!("{str}");

    Ok(VmValue::Void)
}

pub fn native_panic(val: &[VmValue]) -> Result<VmValue, ErrType> {
    let VmValue::Str(str) = &val[0] else { panic!("function called with wrong argument...") };
    println!("{str}");

    Err(ErrType::RuntimeError { msg: str.clone() })
}

pub fn native_str_len(val: &[VmValue]) -> Result<VmValue, ErrType> {
    let VmValue::Str(str) = &val[0] else { panic!("function called with wrong argument...") };
    Ok(VmValue::Int(str.len().try_into().unwrap()))
}