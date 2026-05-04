use std::{collections::BTreeMap, env, error::Error, fmt::Write as _, fs, path::PathBuf};

use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use serde_json::Value;

const DOC_LINKS_FILE: &str = "doc_links.ts";
const SCHEMA_URL: &str = "https://turborepo.dev/schema.json";

#[derive(Debug, Deserialize)]
struct HoverSpec {
    #[serde(rename = "docsKey")]
    docs_key: String,
    context: String,
    example: String,
    #[serde(rename = "summaryOverride")]
    summary_override: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={DOC_LINKS_FILE}");

    let source = fs::read_to_string(DOC_LINKS_FILE)?;
    let doc_links = parse_string_map(&source, "DOC_LINKS")?;
    let top_level_keys = parse_string_array(&source, "topLevel")?;
    let task_field_keys = parse_string_array(&source, "taskFields")?;
    let top_level_hovers =
        parse_json_const::<BTreeMap<String, HoverSpec>>(&source, "TOP_LEVEL_HOVERS_JSON")?;
    let task_field_hovers =
        parse_json_const::<BTreeMap<String, HoverSpec>>(&source, "TASK_FIELD_HOVERS_JSON")?;

    let client = Client::builder()
        .user_agent("turbo-lsp-doc-checker")
        .build()?;
    validate_doc_links(&client, &doc_links)?;
    validate_hover_specs(
        &doc_links,
        &top_level_keys,
        &task_field_keys,
        &top_level_hovers,
        &task_field_hovers,
    )?;
    let schema = fetch_schema(&client)?;
    validate_schema_keys(&schema, &top_level_keys, &task_field_keys)?;

    let top_level_summaries = schema_summaries(&schema, &top_level_keys, SchemaSection::TopLevel)?;
    let task_field_summaries =
        schema_summaries(&schema, &task_field_keys, SchemaSection::TaskFields)?;

    write_generated_module(
        &doc_links,
        &top_level_hovers,
        &task_field_hovers,
        &top_level_summaries,
        &task_field_summaries,
    )?;

    Ok(())
}

fn parse_string_map(
    source: &str,
    const_name: &str,
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let block = extract_brace_block(source, const_name)?;
    let mut map = BTreeMap::new();
    let mut pending_key: Option<String> = None;

    for raw_line in block.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(key) = &pending_key {
            if let Some(value) = parse_quoted_value(line) {
                map.insert(key.clone(), value);
                pending_key = None;
            }
            continue;
        }

        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_string();
        let rest = rest.trim();

        if let Some(value) = parse_quoted_value(rest) {
            map.insert(key, value);
        } else {
            pending_key = Some(key);
        }
    }

    if let Some(key) = pending_key {
        return Err(
            format!("unterminated string value for key `{key}` in {DOC_LINKS_FILE}").into(),
        );
    }
    if map.is_empty() {
        return Err(format!("no doc links parsed from {DOC_LINKS_FILE}").into());
    }

    Ok(map)
}

fn parse_string_array(source: &str, array_name: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let block = extract_bracket_block(source, array_name)?;
    let values = block
        .lines()
        .filter_map(|line| parse_quoted_value(line.trim()))
        .collect::<Vec<_>>();

    if values.is_empty() {
        return Err(
            format!("no values parsed for array `{array_name}` in {DOC_LINKS_FILE}").into(),
        );
    }

    Ok(values)
}

fn parse_json_const<T: for<'de> Deserialize<'de>>(
    source: &str,
    const_name: &str,
) -> Result<T, Box<dyn Error>> {
    let marker = format!("export const {const_name} = String.raw`");
    let start = source
        .find(&marker)
        .ok_or_else(|| format!("missing `{const_name}` const in {DOC_LINKS_FILE}"))?;
    let rest = &source[start + marker.len()..];
    let end = rest
        .find("`;")
        .ok_or_else(|| format!("unterminated `{const_name}` const in {DOC_LINKS_FILE}"))?;
    Ok(serde_json::from_str(&rest[..end])?)
}

fn extract_brace_block<'a>(source: &'a str, const_name: &str) -> Result<&'a str, Box<dyn Error>> {
    let marker = format!("export const {const_name} = {{");
    let start = source
        .find(&marker)
        .ok_or_else(|| format!("missing `{const_name}` block in {DOC_LINKS_FILE}"))?;
    let rest = &source[start + marker.len()..];
    let end = rest
        .find("} as const;")
        .ok_or_else(|| format!("unterminated `{const_name}` block in {DOC_LINKS_FILE}"))?;
    Ok(&rest[..end])
}

fn extract_bracket_block<'a>(source: &'a str, array_name: &str) -> Result<&'a str, Box<dyn Error>> {
    let marker = format!("{array_name}: [");
    let start = source
        .find(&marker)
        .ok_or_else(|| format!("missing `{array_name}` array in {DOC_LINKS_FILE}"))?;
    let rest = &source[start + marker.len()..];
    let end = rest
        .find(']')
        .ok_or_else(|| format!("unterminated `{array_name}` array in {DOC_LINKS_FILE}"))?;
    Ok(&rest[..end])
}

fn parse_quoted_value(input: &str) -> Option<String> {
    let start = input.find('"')?;
    let rest = &input[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn validate_doc_links(
    client: &Client,
    doc_links: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let mut page_cache = BTreeMap::<String, String>::new();

    for (name, raw_url) in doc_links {
        let url = Url::parse(raw_url)?;
        let fragment = url.fragment().map(str::to_string);
        let mut base = url;
        base.set_fragment(None);
        let base_url = base.to_string();

        if !page_cache.contains_key(&base_url) {
            let body = client.get(&base_url).send()?.error_for_status()?.text()?;
            page_cache.insert(base_url.clone(), body);
        }

        if let Some(anchor) = fragment {
            let body = page_cache
                .get(&base_url)
                .ok_or_else(|| format!("missing cached page body for {base_url}"))?;
            if !anchor_exists(body, &anchor) {
                return Err(format!(
                    "bad doc link `{name}`: missing anchor `#{anchor}` in {base_url}"
                )
                .into());
            }
        }
    }

    Ok(())
}

fn anchor_exists(body: &str, anchor: &str) -> bool {
    [
        format!("[#{anchor}]"),
        format!("id=\"{anchor}\""),
        format!("id='{anchor}'"),
        format!("href=\"#{anchor}\""),
        format!("href='#{anchor}'"),
    ]
    .iter()
    .any(|needle| body.contains(needle))
}

fn validate_hover_specs(
    doc_links: &BTreeMap<String, String>,
    top_level_keys: &[String],
    task_field_keys: &[String],
    top_level_hovers: &BTreeMap<String, HoverSpec>,
    task_field_hovers: &BTreeMap<String, HoverSpec>,
) -> Result<(), Box<dyn Error>> {
    for key in top_level_keys {
        if !top_level_hovers.contains_key(key) {
            return Err(format!("missing top-level hover spec `{key}`").into());
        }
    }
    for key in task_field_keys {
        if !task_field_hovers.contains_key(key) {
            return Err(format!("missing task-field hover spec `{key}`").into());
        }
    }

    for (key, spec) in top_level_hovers {
        if !doc_links.contains_key(&spec.docs_key) {
            return Err(format!(
                "top-level hover `{key}` references unknown docsKey `{}`",
                spec.docs_key
            )
            .into());
        }
    }
    for (key, spec) in task_field_hovers {
        if !doc_links.contains_key(&spec.docs_key) {
            return Err(format!(
                "task-field hover `{key}` references unknown docsKey `{}`",
                spec.docs_key
            )
            .into());
        }
    }

    Ok(())
}

fn fetch_schema(client: &Client) -> Result<Value, Box<dyn Error>> {
    Ok(client
        .get(SCHEMA_URL)
        .send()?
        .error_for_status()?
        .json::<Value>()?)
}

fn validate_schema_keys(
    schema: &Value,
    top_level_keys: &[String],
    task_field_keys: &[String],
) -> Result<(), Box<dyn Error>> {
    let top_level = schema["properties"]
        .as_object()
        .ok_or("schema missing top-level properties")?;
    for key in top_level_keys {
        if key == "pipeline" {
            continue;
        }
        if !top_level.contains_key(key) {
            return Err(format!("schema missing top-level key `{key}`").into());
        }
    }

    let task_fields = schema["definitions"]["Pipeline"]["properties"]
        .as_object()
        .ok_or("schema missing Pipeline.properties")?;
    for key in task_field_keys {
        if !task_fields.contains_key(key) {
            return Err(format!("schema missing task field `{key}`").into());
        }
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum SchemaSection {
    TopLevel,
    TaskFields,
}

fn schema_summaries(
    schema: &Value,
    keys: &[String],
    section: SchemaSection,
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let properties = match section {
        SchemaSection::TopLevel => schema["properties"].as_object(),
        SchemaSection::TaskFields => schema["definitions"]["Pipeline"]["properties"].as_object(),
    }
    .ok_or("schema missing properties for summary extraction")?;

    let mut summaries = BTreeMap::new();
    for key in keys {
        if key == "pipeline" {
            continue;
        }
        let Some(node) = properties.get(key) else {
            continue;
        };
        let Some(description) = node.get("description").and_then(Value::as_str) else {
            continue;
        };
        summaries.insert(key.clone(), normalize_description(description));
    }

    Ok(summaries)
}

fn normalize_description(description: &str) -> String {
    let first_paragraph = description.split("\n\n").next().unwrap_or(description);
    first_paragraph
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_generated_module(
    doc_links: &BTreeMap<String, String>,
    top_level_hovers: &BTreeMap<String, HoverSpec>,
    task_field_hovers: &BTreeMap<String, HoverSpec>,
    top_level_summaries: &BTreeMap<String, String>,
    task_field_summaries: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let out_path = out_dir.join("doc_links_generated.rs");

    let mut generated = String::new();
    generated.push_str("pub(crate) mod docs {\n");
    for (name, url) in doc_links {
        let const_name = const_name(name);
        let _ = writeln!(generated, "    pub const {const_name}: &str = {url:?};");
    }
    generated.push_str("}\n\n");
    generated.push_str("#[derive(Clone, Copy)]\n");
    generated.push_str("pub(crate) struct HoverMeta {\n");
    generated.push_str("    pub(crate) docs_url: &'static str,\n");
    generated.push_str("    pub(crate) context: &'static str,\n");
    generated.push_str("    pub(crate) example: &'static str,\n");
    generated.push_str("    pub(crate) summary_override: Option<&'static str>,\n");
    generated.push_str("}\n\n");

    write_hover_meta_fn(&mut generated, "top_level_hover_meta", top_level_hovers);
    write_hover_meta_fn(&mut generated, "task_field_hover_meta", task_field_hovers);
    write_summary_fn(&mut generated, "top_level_summary", top_level_summaries);
    write_summary_fn(&mut generated, "task_field_summary", task_field_summaries);

    fs::write(out_path, generated)?;
    Ok(())
}

fn write_hover_meta_fn(
    generated: &mut String,
    fn_name: &str,
    entries: &BTreeMap<String, HoverSpec>,
) {
    let _ = writeln!(
        generated,
        "pub(crate) fn {fn_name}(name: &str) -> Option<HoverMeta> {{"
    );
    generated.push_str("    match name {\n");
    for (key, spec) in entries {
        let docs_const = const_name(&spec.docs_key);
        let _ = writeln!(
            generated,
            "        {key:?} => Some(HoverMeta {{ docs_url: docs::{docs_const}, context: {context:?}, example: {example:?}, summary_override: {summary_override} }}),",
            context = spec.context,
            example = spec.example,
            summary_override = option_str_literal(spec.summary_override.as_deref()),
        );
    }
    generated.push_str("        _ => None,\n    }\n}\n\n");
}

fn write_summary_fn(generated: &mut String, fn_name: &str, entries: &BTreeMap<String, String>) {
    let _ = writeln!(
        generated,
        "pub(crate) fn {fn_name}(name: &str) -> Option<&'static str> {{"
    );
    generated.push_str("    match name {\n");
    for (key, summary) in entries {
        let _ = writeln!(generated, "        {key:?} => Some({summary:?}),");
    }
    generated.push_str("        _ => None,\n    }\n}\n");
}

fn option_str_literal(value: Option<&str>) -> String {
    value.map_or_else(|| "None".to_string(), |value| format!("Some({value:?})"))
}

fn const_name(name: &str) -> String {
    let mut output = String::new();
    for character in name.chars() {
        if character.is_ascii_uppercase() && !output.is_empty() {
            output.push('_');
        }
        output.push(character.to_ascii_uppercase());
    }
    output
}
