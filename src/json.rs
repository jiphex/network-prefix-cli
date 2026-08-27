//! A 60-line JSON writer, so `--json` does not cost a serde dependency.

pub enum J {
    Str(String),
    /// Pre-rendered numeric literal. Address counts are far too big for a
    /// double, so they are emitted as JSON numbers built from exact digits.
    Num(String),
    Bool(bool),
    Arr(Vec<J>),
    Obj(Vec<(&'static str, J)>),
    Null,
}

pub fn s(v: impl Into<String>) -> J {
    J::Str(v.into())
}

pub fn n(v: impl std::fmt::Display) -> J {
    J::Num(v.to_string())
}

impl J {
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        let pad = |d: usize| "  ".repeat(d);
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Num(v) => out.push_str(v),
            J::Str(v) => {
                out.push('"');
                escape(v, out);
                out.push('"');
            }
            J::Arr(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    item.write(out, depth + 1);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(depth));
                out.push(']');
            }
            J::Obj(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (i, (k, v)) in fields.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    out.push('"');
                    escape(k, out);
                    out.push_str("\": ");
                    v.write(out, depth + 1);
                    if i + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(depth));
                out.push('}');
            }
        }
    }
}

fn escape(v: &str, out: &mut String) {
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_nested_structures() {
        let j = J::Obj(vec![
            ("prefix", s("10.0.0.0/24")),
            ("addresses", n(256u32)),
            ("private", J::Bool(true)),
            ("subnets", J::Arr(vec![s("10.0.0.0/25")])),
            ("parent", J::Null),
        ]);
        assert_eq!(
            j.render(),
            "{\n  \"prefix\": \"10.0.0.0/24\",\n  \"addresses\": 256,\n  \"private\": true,\n  \"subnets\": [\n    \"10.0.0.0/25\"\n  ],\n  \"parent\": null\n}\n"
        );
    }

    #[test]
    fn escapes_strings() {
        assert_eq!(s("a\"b\\c\nd").render(), "\"a\\\"b\\\\c\\nd\"\n");
        assert_eq!(s("\u{1}").render(), "\"\\u0001\"\n");
    }

    #[test]
    fn empty_containers_stay_on_one_line() {
        assert_eq!(J::Arr(vec![]).render(), "[]\n");
        assert_eq!(J::Obj(vec![]).render(), "{}\n");
    }
}
