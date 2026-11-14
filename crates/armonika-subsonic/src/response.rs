//! JSON and XML envelopes of the responses.
//!
//! Handlers return a [`Payload`]: the name of the node and its content as a JSON value.
//! The JSON response puts the value under that name. The XML response converts it with
//! fixed rules: scalar members become attributes, objects become child elements,
//! arrays become repeated elements named after the key, and the member `value` is the
//! text of the element.

use std::fmt::Write;
use std::time::Duration;

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::error::ApiError;

/// Protocol version announced to clients.
pub const API_VERSION: &str = "1.16.1";

/// Value of `type` in every response.
pub const SERVER_TYPE: &str = "armonika";

/// Namespace of the XML responses.
pub const XML_NAMESPACE: &str = "http://subsonic.org/restapi";

/// What a handler answers: the node name (for example `"album"`) and its content.
#[derive(Clone, Debug, PartialEq)]
pub struct Payload(pub &'static str, pub Value);

impl Payload {
    /// A response without a node, as `ping` answers.
    pub fn empty() -> Payload {
        Payload("", Value::Null)
    }
}

/// The format asked with `f`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xml,
    Json,
}

impl Format {
    /// `json` gives JSON (so does `jsonp`, which is refused with an error in JSON); anything else, including no value, gives XML.
    pub fn from_param(f: Option<&str>) -> Format {
        match f {
            Some("json" | "jsonp") => Format::Json,
            _ => Format::Xml,
        }
    }
}

/// Builds the HTTP response, with the `Server-Timing` header for `elapsed`.
pub fn render(format: Format, result: Result<Payload, ApiError>, elapsed: Duration) -> Response {
    let (content_type, body) = match format {
        Format::Json => ("application/json; charset=utf-8", to_json(&result)),
        Format::Xml => ("text/xml; charset=utf-8", to_xml(&result)),
    };
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    let timing = format!("app;dur={:.1}", elapsed.as_secs_f64() * 1000.0);
    if let Ok(value) = HeaderValue::from_str(&timing) {
        headers.insert("server-timing", value);
    }
    response
}

fn to_json(result: &Result<Payload, ApiError>) -> String {
    let mut body = Map::new();
    body.insert("status".into(), json!(status(result)));
    body.insert("version".into(), json!(API_VERSION));
    body.insert("type".into(), json!(SERVER_TYPE));
    body.insert("serverVersion".into(), json!(armonika_core::VERSION));
    body.insert("openSubsonic".into(), json!(true));
    match result {
        Ok(Payload(name, value)) if !name.is_empty() && !value.is_null() => {
            body.insert((*name).to_owned(), value.clone());
        }
        Ok(_) => {}
        Err(e) => {
            body.insert(
                "error".into(),
                json!({ "code": e.code, "message": e.message }),
            );
        }
    }
    json!({ "subsonic-response": body }).to_string()
}

fn to_xml(result: &Result<Payload, ApiError>) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = write!(
        out,
        "<subsonic-response xmlns=\"{XML_NAMESPACE}\" status=\"{}\" version=\"{API_VERSION}\" \
         type=\"{SERVER_TYPE}\" serverVersion=\"{}\" openSubsonic=\"true\">",
        status(result),
        escape(armonika_core::VERSION),
    );
    match result {
        Ok(Payload(name, value)) if !name.is_empty() => write_element(&mut out, name, value),
        Ok(_) => {}
        Err(e) => {
            let _ = write!(
                out,
                "<error code=\"{}\" message=\"{}\"/>",
                e.code,
                escape(&e.message)
            );
        }
    }
    out.push_str("</subsonic-response>");
    out
}

fn status(result: &Result<Payload, ApiError>) -> &'static str {
    if result.is_ok() { "ok" } else { "failed" }
}

/// The text of a scalar, `None` for null and for containers.
fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn write_element(out: &mut String, name: &str, v: &Value) {
    match v {
        Value::Null => {}
        Value::Array(items) => items.iter().for_each(|i| write_element(out, name, i)),
        Value::Object(map) => {
            let _ = write!(out, "<{name}");
            let mut children = Vec::new();
            for (key, member) in map {
                match member {
                    Value::Object(_) | Value::Array(_) => children.push((key, member)),
                    _ if key == "value" => {}
                    _ => {
                        if let Some(text) = scalar_text(member) {
                            let _ = write!(out, " {key}=\"{}\"", escape(&text));
                        }
                    }
                }
            }
            let text = map.get("value").and_then(scalar_text);
            if children.is_empty() && text.is_none() {
                out.push_str("/>");
                return;
            }
            out.push('>');
            if let Some(text) = text {
                out.push_str(&escape(&text));
            }
            for (key, member) in children {
                write_element(out, key, member);
            }
            let _ = write!(out, "</{name}>");
        }
        scalar => {
            if let Some(text) = scalar_text(scalar) {
                let _ = write!(out, "<{name}>{}</{name}>", escape(&text));
            }
        }
    }
}

/// Escapes text for an attribute or element, dropping characters XML cannot hold.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_rules_for_attributes_children_arrays_and_value() {
        let v = json!({
            "id": "a&b",
            "tags": [{"value": "x<y"}, {"value": "z"}],
            "child": {"n": 3, "ok": true},
            "missing": null,
        });
        let mut out = String::new();
        write_element(&mut out, "node", &v);
        assert_eq!(
            out,
            "<node id=\"a&amp;b\"><child n=\"3\" ok=\"true\"/>\
             <tags>x&lt;y</tags><tags>z</tags></node>"
        );
    }

    #[test]
    fn scalar_arrays_repeat_the_element() {
        let mut out = String::new();
        write_element(&mut out, "versions", &json!([1, 2]));
        assert_eq!(out, "<versions>1</versions><versions>2</versions>");
    }
}
