use js_sys::{Array, JsString, Object, Reflect, Set};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use crate::AdocWeaveError;
use crate::request_conversion::invalid_request;

const MAX_DEPTH: u16 = 128;
const MAX_ARRAY_LENGTH: u32 = 20_000;
const MAX_OBJECT_KEYS: u32 = 20_000;
const MAX_TOTAL_NODES: u32 = 100_000;
const MAX_TOTAL_KEYS: u32 = 100_000;
const MAX_STRING_UTF16_UNITS: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_STRING_UTF16_UNITS: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_STRING_UTF8_BYTES: u64 = 32 * 1024 * 1024;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Array, js_name = isArray, catch)]
    fn checked_is_array(value: &JsValue) -> Result<bool, JsValue>;
}

#[derive(Default)]
struct Budget {
    nodes: u32,
    keys: u32,
    string_utf16_units: u64,
    string_utf8_bytes: u64,
}

pub(crate) fn preflight(value: &JsValue) -> Result<(), AdocWeaveError> {
    let ancestors = Set::new(&JsValue::UNDEFINED);
    let plain_object_prototype = prototype_of(&Object::new().into())?;
    let plain_array_prototype = prototype_of(&Array::new().into())?;
    inspect(
        value,
        &ancestors,
        plain_object_prototype.as_ref(),
        plain_array_prototype.as_ref(),
        0,
        false,
        false,
        &mut Budget::default(),
    )
}

fn inspect(
    value: &JsValue,
    ancestors: &Set,
    plain_object_prototype: &JsValue,
    plain_array_prototype: &JsValue,
    depth: u16,
    allow_null: bool,
    map_null_values: bool,
    budget: &mut Budget,
) -> Result<(), AdocWeaveError> {
    if depth >= MAX_DEPTH {
        return Err(limit_error("request nesting depth"));
    }
    budget.nodes = budget
        .nodes
        .checked_add(1)
        .filter(|count| *count <= MAX_TOTAL_NODES)
        .ok_or_else(|| limit_error("request node count"))?;

    if value.is_string() {
        return inspect_string(value.unchecked_ref(), budget);
    }
    if value.is_null() {
        return if allow_null {
            Ok(())
        } else {
            Err(invalid_request(
                "null is not allowed for this request field",
            ))
        };
    }
    if value.is_undefined() || value.as_bool().is_some() || value.as_f64().is_some() {
        return Ok(());
    }
    if value.is_symbol() || value.is_function() || !value.is_object() {
        return Err(invalid_request(
            "request contains an unsupported JavaScript value",
        ));
    }

    let is_array = checked_is_array(value)
        .map_err(|_| invalid_request("request object type could not be inspected"))?;
    if ancestors.has(value) {
        return Err(invalid_request("request must not contain a cycle"));
    }
    ancestors.add(value);
    let result = if is_array {
        inspect_array(
            value,
            ancestors,
            plain_object_prototype,
            plain_array_prototype,
            depth,
            map_null_values,
            budget,
        )
    } else {
        inspect_object(
            value,
            ancestors,
            plain_object_prototype,
            plain_array_prototype,
            depth,
            map_null_values,
            budget,
        )
    };
    ancestors.delete(value);
    result
}

fn inspect_string(value: &JsString, budget: &mut Budget) -> Result<(), AdocWeaveError> {
    let utf16_units = u64::from(value.length());
    if utf16_units > MAX_STRING_UTF16_UNITS {
        return Err(limit_error("request string length"));
    }
    budget.string_utf16_units = budget
        .string_utf16_units
        .checked_add(utf16_units)
        .filter(|count| *count <= MAX_TOTAL_STRING_UTF16_UNITS)
        .ok_or_else(|| limit_error("request string length"))?;

    let utf8_bytes = char::decode_utf16(value.iter())
        .map(|character| character.unwrap_or(char::REPLACEMENT_CHARACTER).len_utf8() as u64)
        .try_fold(0_u64, |total, bytes| total.checked_add(bytes))
        .ok_or_else(|| limit_error("request string bytes"))?;
    budget.string_utf8_bytes = budget
        .string_utf8_bytes
        .checked_add(utf8_bytes)
        .filter(|count| *count <= MAX_TOTAL_STRING_UTF8_BYTES)
        .ok_or_else(|| limit_error("request string bytes"))?;
    Ok(())
}

fn inspect_array(
    value: &JsValue,
    ancestors: &Set,
    plain_object_prototype: &JsValue,
    plain_array_prototype: &JsValue,
    depth: u16,
    _map_null_values: bool,
    budget: &mut Budget,
) -> Result<(), AdocWeaveError> {
    let prototype = prototype_of(value)?;
    if !Object::is(prototype.as_ref(), plain_array_prototype) {
        return Err(invalid_request("request arrays must be plain arrays"));
    }
    let length = Reflect::get(value, &JsValue::from_str("length"))
        .map_err(|_| invalid_request("request array length could not be read"))?
        .as_f64()
        .filter(|length| length.is_finite() && *length >= 0.0 && length.fract() == 0.0)
        .ok_or_else(|| invalid_request("request array length is invalid"))?;
    if length > f64::from(MAX_ARRAY_LENGTH) {
        return Err(limit_error("request array length"));
    }
    let length = length as u32;
    inspect_array_keys(value, length, budget)?;
    for index in 0..length {
        let element = Reflect::get_u32(value, index)
            .map_err(|_| invalid_request("request array element could not be read"))?;
        inspect(
            &element,
            ancestors,
            plain_object_prototype,
            plain_array_prototype,
            depth + 1,
            false,
            false,
            budget,
        )?;
    }
    Ok(())
}

fn inspect_array_keys(
    value: &JsValue,
    length: u32,
    budget: &mut Budget,
) -> Result<(), AdocWeaveError> {
    let keys = own_keys(value, budget)?;
    for key in keys.iter() {
        let Some(key_text) = key.as_string() else {
            return Err(invalid_request(
                "request arrays must not have symbol properties",
            ));
        };
        if key_text == "length" {
            continue;
        }
        key_text
            .parse::<u32>()
            .ok()
            .filter(|index| index.to_string() == key_text && *index < length)
            .ok_or_else(|| invalid_request("request arrays must not have custom properties"))?;
        inspect_data_property(value, &key, true)?;
    }
    Ok(())
}

fn inspect_object(
    value: &JsValue,
    ancestors: &Set,
    plain_object_prototype: &JsValue,
    plain_array_prototype: &JsValue,
    depth: u16,
    map_null_values: bool,
    budget: &mut Budget,
) -> Result<(), AdocWeaveError> {
    let prototype = prototype_of(value)?;
    let prototype_value: &JsValue = prototype.as_ref();
    if !prototype_value.is_null() && !Object::is(prototype_value, plain_object_prototype) {
        return Err(invalid_request("request objects must be plain objects"));
    }
    let keys = own_keys(value, budget)?;
    for key in keys.iter() {
        let Some(key_text) = key.as_string() else {
            return Err(invalid_request(
                "request objects must not have symbol properties",
            ));
        };
        inspect_data_property(value, &key, true)?;
        let field = Reflect::get(value, &key)
            .map_err(|_| invalid_request("request field could not be read"))?;
        inspect(
            &field,
            ancestors,
            plain_object_prototype,
            plain_array_prototype,
            depth + 1,
            map_null_values || key_text == "bibliography",
            matches!(key_text.as_str(), "attributes" | "protectedAttributes"),
            budget,
        )?;
    }
    Ok(())
}

fn own_keys(value: &JsValue, budget: &mut Budget) -> Result<Array, AdocWeaveError> {
    let keys = Reflect::own_keys(value)
        .map_err(|_| invalid_request("request object keys could not be inspected"))?;
    if keys.length() > MAX_OBJECT_KEYS {
        return Err(limit_error("request object key count"));
    }
    budget.keys = budget
        .keys
        .checked_add(keys.length())
        .filter(|count| *count <= MAX_TOTAL_KEYS)
        .ok_or_else(|| limit_error("request object key count"))?;
    Ok(keys)
}

fn inspect_data_property(
    value: &JsValue,
    key: &JsValue,
    require_enumerable: bool,
) -> Result<(), AdocWeaveError> {
    let descriptor = Reflect::get_own_property_descriptor(value.unchecked_ref::<Object>(), key)
        .map_err(|_| invalid_request("request property could not be inspected"))?;
    if descriptor.is_undefined() {
        return Err(invalid_request(
            "request property changed during inspection",
        ));
    }
    let getter = Reflect::get(&descriptor, &JsValue::from_str("get"))
        .map_err(|_| invalid_request("request property descriptor could not be read"))?;
    let setter = Reflect::get(&descriptor, &JsValue::from_str("set"))
        .map_err(|_| invalid_request("request property descriptor could not be read"))?;
    if !getter.is_undefined() || !setter.is_undefined() {
        return Err(invalid_request(
            "request properties must be data properties",
        ));
    }
    if require_enumerable {
        let enumerable = Reflect::get(&descriptor, &JsValue::from_str("enumerable"))
            .map_err(|_| invalid_request("request property descriptor could not be read"))?;
        if enumerable.as_bool() != Some(true) {
            return Err(invalid_request("request properties must be enumerable"));
        }
    }
    Ok(())
}

fn prototype_of(value: &JsValue) -> Result<Object, AdocWeaveError> {
    Reflect::get_prototype_of(value)
        .map_err(|_| invalid_request("request object prototype could not be inspected"))
}

fn limit_error(resource: &str) -> AdocWeaveError {
    AdocWeaveError {
        code: "input-limit-exceeded".to_owned(),
        message: format!("{resource} exceeds the fixed WebAssembly boundary limit"),
    }
}
