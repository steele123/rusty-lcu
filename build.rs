use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::Path,
};

use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use serde_json::Value;

fn main() {
    println!("cargo:rerun-if-env-changed=LCU_SWAGGER_PATH");
    println!("cargo:rerun-if-changed=schema/swagger.json");

    let swagger_path =
        env::var("LCU_SWAGGER_PATH").unwrap_or_else(|_| "schema/swagger.json".to_string());

    let swagger = fs::read_to_string(&swagger_path).unwrap_or_else(|error| {
        panic!(
            "failed to read {swagger_path}: {error}. Download it from https://raw.githubusercontent.com/dysolix/hasagi-types/main/swagger.json"
        )
    });

    let document: Value = serde_json::from_str(&swagger).expect("valid OpenAPI JSON");
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .expect("OpenAPI document with paths");
    let schemas = document
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let schema_info = SchemaInfo::from_document(&document);

    let mut endpoints = Vec::new();
    let methods = ["delete", "get", "patch", "post", "put"];

    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };

        for method in methods {
            let Some(operation) = item.get(method).and_then(Value::as_object) else {
                continue;
            };

            let operation_id = operation
                .get("operationId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| fallback_operation_id(method, path));

            endpoints.push(Operation {
                method: method.to_ascii_uppercase(),
                path: path.clone(),
                operation_id,
                path_params: parameter_names(operation, "path"),
                query_params: parameter_names(operation, "query"),
                required_query_params: required_parameter_names(operation, "query"),
                has_body: operation.get("requestBody").is_some(),
                request_type: request_type(operation),
                response_type: response_type(operation),
                tags: operation
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|tags| {
                        tags.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            });
        }
    }

    endpoints.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.method.cmp(&right.method))
            .then(left.operation_id.cmp(&right.operation_id))
    });

    let generated = generate(&endpoints, &schemas, &schema_info);
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set");
    fs::write(Path::new(&out_dir).join("lcu_endpoints.rs"), generated)
        .expect("write generated endpoints");
}

#[derive(Debug)]
struct Operation {
    method: String,
    path: String,
    operation_id: String,
    path_params: Vec<String>,
    query_params: Vec<String>,
    required_query_params: Vec<String>,
    has_body: bool,
    request_type: Option<String>,
    response_type: Option<String>,
    tags: Vec<String>,
}

#[derive(Debug)]
struct SchemaInfo {
    title: String,
    version: String,
    upstream_url: String,
}

impl SchemaInfo {
    fn from_document(document: &Value) -> Self {
        Self {
            title: document
                .get("info")
                .and_then(|info| info.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("LCU SCHEMA")
                .to_string(),
            version: document
                .get("info")
                .and_then(|info| info.get("version"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            upstream_url:
                "https://raw.githubusercontent.com/dysolix/hasagi-types/main/swagger.json"
                    .to_string(),
        }
    }
}

fn parameter_names(operation: &serde_json::Map<String, Value>, location: &str) -> Vec<String> {
    operation
        .get("parameters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|parameter| parameter.get("in").and_then(Value::as_str) == Some(location))
        .filter_map(|parameter| parameter.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn required_parameter_names(
    operation: &serde_json::Map<String, Value>,
    location: &str,
) -> Vec<String> {
    operation
        .get("parameters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|parameter| parameter.get("in").and_then(Value::as_str) == Some(location))
        .filter(|parameter| parameter.get("required").and_then(Value::as_bool) == Some(true))
        .filter_map(|parameter| parameter.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn fallback_operation_id(method: &str, path: &str) -> String {
    let path_name = path
        .trim_matches('/')
        .replace(['/', '{', '}'], "_")
        .replace('-', "_");
    format!("{method}_{path_name}")
}

fn request_type(operation: &serde_json::Map<String, Value>) -> Option<String> {
    operation
        .get("requestBody")
        .and_then(|body| body.get("content"))
        .and_then(json_schema_from_content)
        .and_then(ref_type_name)
}

fn response_type(operation: &serde_json::Map<String, Value>) -> Option<String> {
    let responses = operation.get("responses")?.as_object()?;
    ["2XX", "200", "201", "202", "204"]
        .iter()
        .filter_map(|status| responses.get(*status))
        .filter_map(|response| response.get("content"))
        .filter_map(json_schema_from_content)
        .find_map(ref_type_name)
}

fn json_schema_from_content(content: &Value) -> Option<&Value> {
    let content = content.as_object()?;
    content
        .get("application/json")
        .or_else(|| content.values().next())
        .and_then(|media_type| media_type.get("schema"))
}

fn ref_type_name(schema: &Value) -> Option<String> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.rsplit('/').next())
        .map(type_name)
}

fn generate(
    endpoints: &[Operation],
    schemas: &serde_json::Map<String, Value>,
    schema_info: &SchemaInfo,
) -> String {
    let mut used_functions = HashMap::<String, usize>::new();
    let mut output = generate_models(schemas);
    let tags = unique_tags(endpoints);
    output.push_str(&format!(
        "pub const SCHEMA_TITLE: &str = {};\npub const SCHEMA_VERSION: &str = {};\npub const SCHEMA_UPSTREAM_URL: &str = {};\n\n",
        rust_string(&schema_info.title),
        rust_string(&schema_info.version),
        rust_string(&schema_info.upstream_url),
    ));
    output.push_str(
        r#"#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub path_params: &'static [&'static str],
    pub query_params: &'static [&'static str],
    pub required_query_params: &'static [&'static str],
    pub has_body: bool,
    pub request_type: Option<&'static str>,
    pub response_type: Option<&'static str>,
    pub tags: &'static [&'static str],
}

"#,
    );

    for endpoint in endpoints {
        let constant = constant_name(&endpoint.operation_id);
        let path_params = string_slice(&endpoint.path_params);
        let query_params = string_slice(&endpoint.query_params);
        let required_query_params = string_slice(&endpoint.required_query_params);
        let tags = string_slice(&endpoint.tags);

        output.push_str(&format!(
            "pub const {constant}: Endpoint = Endpoint {{ operation_id: {}, method: {}, path: {}, path_params: {path_params}, query_params: {query_params}, required_query_params: {required_query_params}, has_body: {}, request_type: {}, response_type: {}, tags: {tags} }};\n",
            rust_string(&endpoint.operation_id),
            rust_string(&endpoint.method),
            rust_string(&endpoint.path),
            endpoint.has_body,
            option_string(endpoint.request_type.as_deref()),
            option_string(endpoint.response_type.as_deref()),
        ));
    }

    output.push_str("\npub const ENDPOINTS: &[Endpoint] = &[\n");
    for endpoint in endpoints {
        output.push_str(&format!("    {},\n", constant_name(&endpoint.operation_id)));
    }
    output.push_str("];\n\n");

    output.push_str("pub const TAGS: &[&str] = &[\n");
    for tag in tags {
        output.push_str(&format!("    {},\n", rust_string(&tag)));
    }
    output.push_str("];\n\n");

    output.push_str(
        r#"pub fn find_endpoint(method: &str, path: &str) -> Option<&'static Endpoint> {
    ENDPOINTS
        .iter()
        .find(|endpoint| endpoint.method.eq_ignore_ascii_case(method) && path_matches_template(endpoint.path, path))
}

pub fn find_endpoint_by_operation_id(operation_id: &str) -> Option<&'static Endpoint> {
    ENDPOINTS
        .iter()
        .find(|endpoint| endpoint.operation_id == operation_id)
}

pub fn endpoints_for_tag(tag: &str) -> impl Iterator<Item = &'static Endpoint> + '_ {
    ENDPOINTS
        .iter()
        .filter(move |endpoint| endpoint.tags.iter().any(|endpoint_tag| *endpoint_tag == tag))
}

pub fn endpoints_for_path_prefix(prefix: &str) -> impl Iterator<Item = &'static Endpoint> + '_ {
    ENDPOINTS
        .iter()
        .filter(move |endpoint| endpoint.path.starts_with(prefix))
}

pub fn path_matches_template(template: &str, path: &str) -> bool {
    let template_segments = template.trim_matches('/').split('/').collect::<Vec<_>>();
    let path_segments = path.trim_matches('/').split('/').collect::<Vec<_>>();

    if template_segments.len() != path_segments.len() {
        return false;
    }

    template_segments
        .iter()
        .zip(path_segments.iter())
        .all(|(template_segment, path_segment)| {
            (template_segment.starts_with('{') && template_segment.ends_with('}'))
                || template_segment == path_segment
        })
}

"#,
    );

    for endpoint in endpoints {
        let function = unique_function_name(&endpoint.operation_id, &mut used_functions);
        let constant = constant_name(&endpoint.operation_id);
        output.push_str(&format!(
            "pub async fn {function}(client: &crate::LcuClient, params: crate::EndpointParams) -> crate::Result<serde_json::Value> {{\n    client.request_endpoint(&{constant}, params).await\n}}\n\n"
        ));

        if let Some(response_type) = &endpoint.response_type {
            output.push_str(&format!(
                "pub async fn {function}_typed(client: &crate::LcuClient, params: crate::EndpointParams) -> crate::Result<models::{response_type}> {{\n    client.request_endpoint_as(&{constant}, params).await\n}}\n\n"
            ));
        }

        if let Some(request_type) = &endpoint.request_type {
            output.push_str(&format!(
                "pub async fn {function}_with_body(client: &crate::LcuClient, params: crate::EndpointParams, body: &models::{request_type}) -> crate::Result<serde_json::Value> {{\n    client.request_endpoint(&{constant}, params.body(body)?).await\n}}\n\n"
            ));

            if let Some(response_type) = &endpoint.response_type {
                output.push_str(&format!(
                    "pub async fn {function}_with_body_typed(client: &crate::LcuClient, params: crate::EndpointParams, body: &models::{request_type}) -> crate::Result<models::{response_type}> {{\n    client.request_endpoint_as(&{constant}, params.body(body)?).await\n}}\n\n"
                ));
            }
        }
    }

    output
}

fn unique_tags(endpoints: &[Operation]) -> Vec<String> {
    let mut tags = endpoints
        .iter()
        .flat_map(|endpoint| endpoint.tags.iter().cloned())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags
}

fn generate_models(schemas: &serde_json::Map<String, Value>) -> String {
    let mut output = String::from(
        r#"pub mod models {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::collections::HashMap;

"#,
    );

    let mut names = schemas.keys().collect::<Vec<_>>();
    names.sort();

    for name in names {
        let Some(schema) = schemas.get(name).and_then(Value::as_object) else {
            continue;
        };
        let type_name = type_name(name);

        if schema.get("type").and_then(Value::as_str) == Some("object")
            && schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some()
        {
            output.push_str(&generate_struct(&type_name, schema));
        } else {
            let alias_type = schema_type(&Value::Object(schema.clone()));
            output.push_str(&format!("    pub type {type_name} = {alias_type};\n\n"));
        }
    }

    output.push_str("}\n\n");
    output
}

fn generate_struct(name: &str, schema: &serde_json::Map<String, Value>) -> String {
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let mut output = format!(
        "    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n    pub struct {name} {{\n"
    );

    let mut fields = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties checked")
        .iter()
        .collect::<Vec<_>>();
    fields.sort_by(|left, right| left.0.cmp(right.0));

    for (json_name, field_schema) in fields {
        let field_name = field_name(json_name);
        let mut field_type = schema_type(field_schema);
        let is_required = required.contains(json_name.as_str());

        if !is_required {
            field_type = format!("Option<{field_type}>");
        }

        let serde_options = if is_required {
            format!("rename = {}", rust_string(json_name))
        } else {
            format!(
                "rename = {}, default, skip_serializing_if = \"Option::is_none\"",
                rust_string(json_name)
            )
        };
        output.push_str(&format!("        #[serde({serde_options})]\n"));
        output.push_str(&format!("        pub {field_name}: {field_type},\n"));
    }

    output.push_str("    }\n\n");
    output
}

fn schema_type(schema: &Value) -> String {
    if let Some(type_name) = ref_type_name(schema) {
        return type_name;
    }

    if schema.get("nullable").and_then(Value::as_bool) == Some(true) {
        let mut cloned = schema.clone();
        if let Some(object) = cloned.as_object_mut() {
            object.remove("nullable");
        }
        return format!("Option<{}>", schema_type(&cloned));
    }

    if schema.get("oneOf").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("allOf").is_some()
    {
        return "Value".to_string();
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("array") => {
            let item_type = schema
                .get("items")
                .map(schema_type)
                .unwrap_or_else(|| "Value".to_string());
            format!("Vec<{item_type}>")
        }
        Some("boolean") => "bool".to_string(),
        Some("integer") => integer_type(schema),
        Some("number") => "f64".to_string(),
        Some("string") => "String".to_string(),
        Some("object") => {
            if let Some(additional) = schema.get("additionalProperties") {
                if additional == &Value::Bool(true) {
                    return "HashMap<String, Value>".to_string();
                }
                if additional.is_object() {
                    return format!("HashMap<String, {}>", schema_type(additional));
                }
            }
            "Value".to_string()
        }
        _ => "Value".to_string(),
    }
}

fn integer_type(schema: &Value) -> String {
    match schema.get("format").and_then(Value::as_str) {
        Some("uint8") | Some("uint16") | Some("uint32") => "u32".to_string(),
        Some("uint64") => "u64".to_string(),
        Some("int8") | Some("int16") | Some("int32") => "i32".to_string(),
        Some("int64") => "i64".to_string(),
        _ if schema
            .get("minimum")
            .and_then(Value::as_i64)
            .is_some_and(|minimum| minimum >= 0) =>
        {
            "u64".to_string()
        }
        _ => "i64".to_string(),
    }
}

fn rust_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

fn option_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("Some({})", rust_string(value)))
        .unwrap_or_else(|| "None".to_string())
}

fn type_name(value: &str) -> String {
    let mut name = value.to_pascal_case();
    name.retain(|character| character == '_' || character.is_ascii_alphanumeric());
    if name.is_empty() {
        name = "GeneratedModel".to_string();
    }
    if name
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        name.insert_str(0, "Model");
    }
    name
}

fn field_name(value: &str) -> String {
    let keywords = rust_keywords();
    let mut name = value.to_snake_case();
    name.retain(|character| character == '_' || character.is_ascii_alphanumeric());
    if name.is_empty() {
        name = "field".to_string();
    }
    if name
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        name.insert_str(0, "field_");
    }
    if keywords.contains(name.as_str()) {
        name.push('_');
    }
    name
}

fn string_slice(values: &[String]) -> String {
    if values.is_empty() {
        return "&[]".to_string();
    }

    let values = values
        .iter()
        .map(|value| format!("    {},", rust_string(value)))
        .collect::<Vec<_>>()
        .join("\n");

    format!("&[\n{values}\n]")
}

fn constant_name(operation_id: &str) -> String {
    let mut name = operation_id.to_shouty_snake_case();
    name.retain(|character| character == '_' || character.is_ascii_alphanumeric());
    if name
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        name.insert_str(0, "ENDPOINT_");
    }
    name
}

fn unique_function_name(operation_id: &str, used: &mut HashMap<String, usize>) -> String {
    let keywords = rust_keywords();
    let mut name = operation_id.to_snake_case();
    name.retain(|character| character == '_' || character.is_ascii_alphanumeric());
    if name.is_empty() {
        name = "endpoint".to_string();
    }
    if name
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        name.insert_str(0, "endpoint_");
    }
    if keywords.contains(name.as_str()) {
        name.push('_');
    }

    let count = used.entry(name.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        name
    } else {
        format!("{name}_{count}")
    }
}

fn rust_keywords() -> HashSet<&'static str> {
    [
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while",
    ]
    .into_iter()
    .collect()
}
