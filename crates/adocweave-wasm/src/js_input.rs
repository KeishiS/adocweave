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
const MAX_PROPERTY_NAME_UTF16_UNITS: u64 = 1_024;
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

pub(crate) fn preflight(value: &JsValue) -> Result<JsValue, AdocWeaveError> {
    let ancestors = Set::new(&JsValue::UNDEFINED);
    let plain_object_prototype = prototype_of(&Object::new().into())?;
    let plain_array_prototype = prototype_of(&Array::new().into())?;
    snapshot(
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

fn snapshot(
    value: &JsValue,
    ancestors: &Set,
    plain_object_prototype: &JsValue,
    plain_array_prototype: &JsValue,
    depth: u16,
    allow_null: bool,
    map_null_values: bool,
    budget: &mut Budget,
) -> Result<JsValue, AdocWeaveError> {
    if depth >= MAX_DEPTH {
        return Err(limit_error("request nesting depth"));
    }
    budget.nodes = budget
        .nodes
        .checked_add(1)
        .filter(|count| *count <= MAX_TOTAL_NODES)
        .ok_or_else(|| limit_error("request node count"))?;

    if value.is_string() {
        inspect_string(value.unchecked_ref(), MAX_STRING_UTF16_UNITS, budget)?;
        return Ok(value.clone());
    }
    if value.is_null() {
        return if allow_null {
            Ok(value.clone())
        } else {
            Err(invalid_request(
                "null is not allowed for this request field",
            ))
        };
    }
    if value.is_undefined() || value.as_bool().is_some() || value.as_f64().is_some() {
        return Ok(value.clone());
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
        snapshot_array(
            value,
            ancestors,
            plain_object_prototype,
            plain_array_prototype,
            depth,
            budget,
        )
    } else {
        snapshot_object(
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

fn inspect_string(
    value: &JsString,
    maximum_utf16_units: u64,
    budget: &mut Budget,
) -> Result<(), AdocWeaveError> {
    let utf16_units = u64::from(value.length());
    if utf16_units > maximum_utf16_units {
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

fn snapshot_array(
    value: &JsValue,
    ancestors: &Set,
    plain_object_prototype: &JsValue,
    plain_array_prototype: &JsValue,
    depth: u16,
    budget: &mut Budget,
) -> Result<JsValue, AdocWeaveError> {
    let prototype = prototype_of(value)?;
    if !Object::is(prototype.as_ref(), plain_array_prototype) {
        return Err(invalid_request("request arrays must be plain arrays"));
    }
    let keys = own_string_keys(
        value,
        budget,
        "request arrays must not have symbol properties",
    )?;
    let length_key = JsValue::from_str("length");
    let length = data_property_value(value, &length_key, false)?
        .as_f64()
        .filter(|length| length.is_finite() && *length >= 0.0 && length.fract() == 0.0)
        .ok_or_else(|| invalid_request("request array length is invalid"))?;
    if length > f64::from(MAX_ARRAY_LENGTH) {
        return Err(limit_error("request array length"));
    }
    let length = length as u32;
    let result = Array::new_with_length(length);
    let result_object: &Object = result.unchecked_ref();
    for (key, key_text) in keys {
        if key_text == "length" {
            continue;
        }
        key_text
            .parse::<u32>()
            .ok()
            .filter(|index| index.to_string() == key_text && *index < length)
            .ok_or_else(|| invalid_request("request arrays must not have custom properties"))?;
        let field = data_property_value(value, &key, true)?;
        let field = snapshot(
            &field,
            ancestors,
            plain_object_prototype,
            plain_array_prototype,
            depth + 1,
            false,
            false,
            budget,
        )?;
        define_data_property(result_object, &key, &field)?;
    }
    Ok(result.into())
}

fn snapshot_object(
    value: &JsValue,
    ancestors: &Set,
    plain_object_prototype: &JsValue,
    plain_array_prototype: &JsValue,
    depth: u16,
    map_null_values: bool,
    budget: &mut Budget,
) -> Result<JsValue, AdocWeaveError> {
    let prototype = prototype_of(value)?;
    let prototype_value: &JsValue = prototype.as_ref();
    if !prototype_value.is_null() && !Object::is(prototype_value, plain_object_prototype) {
        return Err(invalid_request("request objects must be plain objects"));
    }
    let keys = own_string_keys(
        value,
        budget,
        "request objects must not have symbol properties",
    )?;
    let result = Object::new();
    for (key, key_text) in keys {
        let field = data_property_value(value, &key, true)?;
        if map_null_values && field.is_undefined() {
            return Err(invalid_request(
                "attribute map values must be strings or null",
            ));
        }
        let field = snapshot(
            &field,
            ancestors,
            plain_object_prototype,
            plain_array_prototype,
            depth + 1,
            map_null_values || key_text == "bibliography",
            matches!(key_text.as_str(), "attributes" | "protectedAttributes"),
            budget,
        )?;
        define_data_property(&result, &key, &field)?;
    }
    Ok(result.into())
}

fn own_string_keys(
    value: &JsValue,
    budget: &mut Budget,
    symbol_error: &'static str,
) -> Result<Vec<(JsValue, String)>, AdocWeaveError> {
    // ECMAScript has no lazy own-key API. Reflect.ownKeys necessarily creates the
    // complete key array; the property-count check bounds all subsequent WASM
    // traversal, string conversion, and snapshot allocation.
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

    let mut result = Vec::with_capacity(keys.length() as usize);
    for key in keys.iter() {
        if !key.is_string() {
            return Err(invalid_request(symbol_error));
        }
        let key_string: &JsString = key.unchecked_ref();
        inspect_string(key_string, MAX_PROPERTY_NAME_UTF16_UNITS, budget)?;
        let key_text = key
            .as_string()
            .ok_or_else(|| invalid_request("request property name could not be read"))?;
        result.push((key, key_text));
    }
    Ok(result)
}

fn data_property_value(
    value: &JsValue,
    key: &JsValue,
    require_enumerable: bool,
) -> Result<JsValue, AdocWeaveError> {
    let descriptor = Reflect::get_own_property_descriptor(value.unchecked_ref::<Object>(), key)
        .map_err(|_| invalid_request("request property could not be inspected"))?;
    if descriptor.is_undefined() {
        return Err(invalid_request(
            "request property changed during inspection",
        ));
    }
    let descriptor_object: &Object = descriptor.unchecked_ref();
    let has_getter =
        !Reflect::get_own_property_descriptor(descriptor_object, &JsValue::from_str("get"))
            .map_err(|_| invalid_request("request property descriptor could not be read"))?
            .is_undefined();
    let has_setter =
        !Reflect::get_own_property_descriptor(descriptor_object, &JsValue::from_str("set"))
            .map_err(|_| invalid_request("request property descriptor could not be read"))?
            .is_undefined();
    if has_getter || has_setter {
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
    Reflect::get(&descriptor, &JsValue::from_str("value"))
        .map_err(|_| invalid_request("request property descriptor could not be read"))
}

fn define_data_property(
    target: &Object,
    key: &JsValue,
    value: &JsValue,
) -> Result<(), AdocWeaveError> {
    let null = JsValue::NULL;
    let null_prototype: &Object = null.unchecked_ref();
    let descriptor = Object::create(null_prototype);
    for (name, property_value) in [
        ("value", value.clone()),
        ("writable", JsValue::TRUE),
        ("enumerable", JsValue::TRUE),
        ("configurable", JsValue::TRUE),
    ] {
        Reflect::set(&descriptor, &JsValue::from_str(name), &property_value)
            .map_err(|_| invalid_request("request snapshot could not be created"))?;
    }
    let defined = Reflect::define_property(target, key, &descriptor)
        .map_err(|_| invalid_request("request snapshot could not be created"))?;
    if !defined {
        return Err(invalid_request("request snapshot could not be created"));
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
