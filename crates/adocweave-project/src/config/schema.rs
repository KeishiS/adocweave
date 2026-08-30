use std::fs;
use std::path::Path;

use adocweave::output::diagnostics::{LINT_RULES, LintConfig};
use jsonschema::Draft;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

use super::{
    ProjectConfig, ProjectConfigWire, SCHEMA_VERSION, SyntaxModeWire, default_blank_lines,
};
use crate::ProjectResourceLimits;

const SCHEMA_PATH: &str = "config/adocweave.schema.json";
const SCHEMA_ID: &str = "https://github.com/KeishiS/adocweave/config/adocweave.schema.json";

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate must be inside the repository")
}

fn object_at<'a>(schema: &'a mut Value, pointer: &str) -> &'a mut Map<String, Value> {
    schema
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("generated schema has no {pointer}"))
        .as_object_mut()
        .unwrap_or_else(|| panic!("generated schema value at {pointer} is not an object"))
}

fn replace(schema: &mut Value, pointer: &str, value: Value) {
    *schema
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("generated schema has no {pointer}")) = value;
}

/// TOML has no null value, so an absent optional field must not become JSON null.
fn remove_null_values(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                remove_null_values(value);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                remove_null_values(value);
            }
            if let Some(values) = object.get_mut("enum").and_then(Value::as_array_mut) {
                values.retain(|value| !value.is_null());
            }
            let sole_type = object
                .get_mut("type")
                .and_then(Value::as_array_mut)
                .and_then(|values| {
                    values.retain(|value| value != "null");
                    (values.len() == 1).then(|| values[0].clone())
                });
            if let Some(value) = sole_type {
                object.insert("type".into(), value);
            }
        }
        _ => {}
    }
}

fn generated_schema() -> Value {
    let mut root = SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<ProjectConfigWire>();
    let root = root.as_object_mut().expect("root schema object");
    root.insert("$id".into(), SCHEMA_ID.into());
    root.insert("title".into(), "AdocWeave project configuration".into());
    let mut schema = Value::Object(std::mem::take(root));
    remove_null_values(&mut schema);

    replace(
        &mut schema,
        "/properties/schema-version",
        json!({ "type": "integer", "const": SCHEMA_VERSION }),
    );
    replace(
        &mut schema,
        "/properties/analysis/properties/attributes/additionalProperties",
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "value": { "type": "string" },
                "unset": { "const": true }
            },
            "oneOf": [
                {
                    "required": ["value"],
                    "not": { "required": ["unset"] }
                },
                {
                    "required": ["unset"],
                    "not": { "required": ["value"] }
                }
            ]
        }),
    );
    object_at(&mut schema, "/properties/analysis/properties/attributes")
        .insert("default".into(), json!({}));
    let syntax_default = match SyntaxModeWire::default() {
        SyntaxModeWire::Permissive => "permissive",
        SyntaxModeWire::Strict => "strict",
    };
    object_at(&mut schema, "/properties/analysis/properties/syntax-mode")
        .insert("default".into(), syntax_default.into());

    let lint_defaults = LintConfig::default();
    for (name, default) in [
        ("max-line-length", lint_defaults.max_line_length),
        (
            "max-consecutive-blank-lines",
            lint_defaults.max_consecutive_blank_lines,
        ),
        ("max-diagnostics", lint_defaults.max_diagnostics),
    ] {
        replace(
            &mut schema,
            &format!("/properties/lint/properties/{name}"),
            json!({ "type": "integer", "minimum": 1, "default": default }),
        );
    }
    let mut lint_rules = LINT_RULES
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();
    lint_rules.sort_unstable();
    object_at(&mut schema, "/properties/lint/properties/rules")
        .insert("propertyNames".into(), json!({ "enum": lint_rules }));
    object_at(&mut schema, "/properties/lint/properties/rules").insert("default".into(), json!({}));
    replace(
        &mut schema,
        "/properties/lint/properties/rules/additionalProperties/properties/enabled",
        json!({ "type": "boolean" }),
    );
    let resource_defaults = ProjectResourceLimits::default();
    for (name, maximum) in [
        ("max-files", resource_defaults.max_files as u64),
        ("max-total-bytes", resource_defaults.max_total_bytes),
        ("max-resource-bytes", resource_defaults.max_resource_bytes),
    ] {
        replace(
            &mut schema,
            &format!("/properties/resources/properties/{name}"),
            json!({
                "type": "integer",
                "minimum": 1,
                "maximum": maximum,
                "default": maximum
            }),
        );
    }
    object_at(&mut schema, "/properties/resources/properties/roots")
        .insert("default".into(), json!([]));
    object_at(&mut schema, "/properties/local-targets").insert(
        "allOf".into(),
        json!([{
            "if": {
                "properties": { "enabled": { "const": true } },
                "required": ["enabled"]
            },
            "then": { "required": ["project-root"] }
        }]),
    );

    replace(
        &mut schema,
        "/properties/format/properties/max-consecutive-blank-lines",
        json!({
            "type": "integer",
            "minimum": 1,
            "default": default_blank_lines()
        }),
    );
    replace(
        &mut schema,
        "/properties/html/properties/roles/items",
        json!({ "type": "string", "pattern": "^[A-Za-z0-9_-]+$" }),
    );
    object_at(&mut schema, "/properties/html/properties/roles").insert(
        "description".into(),
        "HTMLへ`role-<name>` classとして出力するblock roleの名前。列挙していないroleは出力しません。".into(),
    );
    object_at(&mut schema, "/properties/html/properties/stylesheet-files")
        .insert("default".into(), json!([]));

    schema
}

fn generated_schema_text() -> String {
    let mut text = serde_json::to_string_pretty(&generated_schema())
        .expect("serialize project configuration schema");
    text.push('\n');
    text
}

#[test]
fn removing_toml_null_keeps_json_schema_keyword_shapes() {
    let mut schema = json!({
        "enum": [null, "only"],
        "type": ["null", "string"]
    });

    remove_null_values(&mut schema);

    assert_eq!(schema, json!({ "enum": ["only"], "type": "string" }));
}

#[test]
fn project_config_schema_is_current_and_valid() {
    let generated = generated_schema_text();
    let checked_in = fs::read_to_string(repository_root().join(SCHEMA_PATH))
        .expect("checked-in project configuration schema")
        .replace("\r\n", "\n");
    assert_eq!(
        checked_in, generated,
        "run `cargo test -p adocweave-project regenerate_project_config_schema -- --ignored`"
    );
    assert!(jsonschema::meta::is_valid(&generated_schema()));
}

#[test]
fn generated_schema_covers_the_configuration_contract() {
    let schema = generated_schema();
    let validator = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .build(&schema)
        .expect("compile generated schema");
    let shared_cases = vec![
        ("minimal", json!({ "schema-version": SCHEMA_VERSION }), true),
        (
            "resource roots",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "roots": ["docs", "docs/api"] } }),
            true,
        ),
        (
            "stylesheet file",
            json!({ "schema-version": SCHEMA_VERSION, "html": { "stylesheet-files": ["styles/manual.css"] } }),
            true,
        ),
        (
            "disabled local targets without root",
            json!({ "schema-version": SCHEMA_VERSION, "local-targets": { "enabled": false } }),
            true,
        ),
        (
            "enabled local targets with root",
            json!({ "schema-version": SCHEMA_VERSION, "local-targets": { "enabled": true, "project-root": "docs" } }),
            true,
        ),
        (
            "equal resource limits",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "max-total-bytes": 1000, "max-resource-bytes": 1000 } }),
            true,
        ),
        (
            "parent resource root",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "roots": ["../escape"] } }),
            false,
        ),
        (
            "absolute resource root",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "roots": ["/abs"] } }),
            false,
        ),
        (
            "resource root with drive name",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "roots": ["C:/docs"] } }),
            false,
        ),
        (
            "resource root with platform-specific separator",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "roots": ["docs\\api"] } }),
            false,
        ),
        (
            "absolute project root",
            json!({ "schema-version": SCHEMA_VERSION, "local-targets": { "project-root": "/abs" } }),
            false,
        ),
        (
            "parent project root",
            json!({ "schema-version": SCHEMA_VERSION, "local-targets": { "project-root": "../escape" } }),
            false,
        ),
        (
            "project root with drive name",
            json!({ "schema-version": SCHEMA_VERSION, "local-targets": { "project-root": "C:/docs" } }),
            false,
        ),
        (
            "project root with platform-specific separator",
            json!({ "schema-version": SCHEMA_VERSION, "local-targets": { "project-root": "docs\\api" } }),
            false,
        ),
        (
            "absolute stylesheet file",
            json!({ "schema-version": SCHEMA_VERSION, "html": { "stylesheet-files": ["/abs/style.css"] } }),
            false,
        ),
        (
            "parent stylesheet file",
            json!({ "schema-version": SCHEMA_VERSION, "html": { "stylesheet-files": ["../style.css"] } }),
            false,
        ),
        (
            "stylesheet file with drive name",
            json!({ "schema-version": SCHEMA_VERSION, "html": { "stylesheet-files": ["C:/styles/manual.css"] } }),
            false,
        ),
        (
            "stylesheet file with platform-specific separator",
            json!({ "schema-version": SCHEMA_VERSION, "html": { "stylesheet-files": ["styles\\manual.css"] } }),
            false,
        ),
        (
            "enabled local targets without root",
            json!({ "schema-version": SCHEMA_VERSION, "local-targets": { "enabled": true } }),
            false,
        ),
        (
            "unknown field",
            json!({ "schema-version": SCHEMA_VERSION, "unknown-section": { "value": 1 } }),
            false,
        ),
        (
            "unsupported schema version",
            json!({ "schema-version": 1 }),
            false,
        ),
    ];

    for (name, config, accepted) in shared_cases {
        assert_eq!(validator.is_valid(&config), accepted, "schema: {name}");
        let source = toml::to_string(&config).expect("convert test configuration to TOML");
        assert_eq!(
            ProjectConfig::parse(&source, Path::new("/workspace")).is_ok(),
            accepted,
            "runtime: {name}"
        );
    }

    let (name, config) = (
        "resource limit exceeds total",
        json!({ "schema-version": SCHEMA_VERSION, "resources": { "max-total-bytes": 1000, "max-resource-bytes": 1001 } }),
    );
    assert!(validator.is_valid(&config), "schema: {name}");
    let source = toml::to_string(&config).expect("convert test configuration to TOML");
    assert!(
        ProjectConfig::parse(&source, Path::new("/workspace")).is_err(),
        "runtime: {name}"
    );
}

#[test]
fn generated_schema_enforces_types_enums_and_single_field_limits() {
    let schema = generated_schema();
    let validator = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .build(&schema)
        .expect("compile generated schema");
    let resource_limits = ProjectResourceLimits::default();
    let invalid = vec![
        ("missing schema version", json!({})),
        ("schema version type", json!({ "schema-version": "2" })),
        (
            "TOML optional field cannot be null",
            json!({ "schema-version": SCHEMA_VERSION, "format": { "newline": null } }),
        ),
        (
            "nested unknown field",
            json!({ "schema-version": SCHEMA_VERSION, "analysis": { "unknown": true } }),
        ),
        (
            "syntax enum",
            json!({ "schema-version": SCHEMA_VERSION, "analysis": { "syntax-mode": "lenient" } }),
        ),
        (
            "severity enum",
            json!({ "schema-version": SCHEMA_VERSION, "lint": { "rules": { "line-too-long": { "severity": "fatal" } } } }),
        ),
        (
            "newline enum",
            json!({ "schema-version": SCHEMA_VERSION, "format": { "newline": "native" } }),
        ),
        (
            "unknown lint rule",
            json!({ "schema-version": SCHEMA_VERSION, "lint": { "rules": { "unknown": {} } } }),
        ),
        (
            "zero lint limit",
            json!({ "schema-version": SCHEMA_VERSION, "lint": { "max-line-length": 0 } }),
        ),
        (
            "resource file ceiling",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "max-files": resource_limits.max_files + 1 } }),
        ),
        (
            "resource total ceiling",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "max-total-bytes": resource_limits.max_total_bytes + 1 } }),
        ),
        (
            "resource item ceiling",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "max-resource-bytes": resource_limits.max_resource_bytes + 1 } }),
        ),
        (
            "zero format limit",
            json!({ "schema-version": SCHEMA_VERSION, "format": { "max-consecutive-blank-lines": 0 } }),
        ),
        (
            "invalid HTML role",
            json!({ "schema-version": SCHEMA_VERSION, "html": { "roles": ["not valid"] } }),
        ),
        (
            "ambiguous attribute",
            json!({ "schema-version": SCHEMA_VERSION, "analysis": { "attributes": { "release": { "value": "draft", "unset": true } } } }),
        ),
    ];
    for (name, config) in invalid {
        assert!(!validator.is_valid(&config), "schema accepted {name}");
    }

    for (name, config) in [
        (
            "resource roots may repeat because the runtime preserves them",
            json!({ "schema-version": SCHEMA_VERSION, "resources": { "roots": ["docs", "docs"] } }),
        ),
        (
            "empty stylesheet URL remains a runtime value",
            json!({ "schema-version": SCHEMA_VERSION, "html": { "stylesheet-urls": [""] } }),
        ),
        (
            "single lint rule field",
            json!({ "schema-version": SCHEMA_VERSION, "lint": { "rules": { "line-too-long": { "enabled": false } } } }),
        ),
    ] {
        assert!(validator.is_valid(&config), "schema rejected {name}");
    }
}

#[test]
#[ignore = "maintainer command that updates the checked-in schema"]
fn regenerate_project_config_schema() {
    fs::write(repository_root().join(SCHEMA_PATH), generated_schema_text())
        .expect("write project configuration schema");
}
