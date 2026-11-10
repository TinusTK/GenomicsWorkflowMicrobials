//! YAML values as the tool commands see them: settings reach the command as
//! text (`S_<setting>`), the same way the v1 Nextflow code passed them.

use serde_yaml::Value;

/// Text of a scalar the way Python's `str()` writes it (v1 parity): booleans
/// `true`/`false`, whole floats keep `.0` (`3.0`), null is empty.
pub fn to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if let Some(f) = n.as_f64().filter(|_| n.is_f64()) {
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            } else {
                n.to_string()
            }
        }
        Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other).unwrap_or_default().trim_end().to_string(),
    }
}

/// Deserialize a scalar (a version like `0.24.0`, `"3.0"` or `1`) as text.
pub fn text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v: Value = serde::Deserialize::deserialize(d)?;
    Ok(to_text(&v))
}

/// Like [`text`], for optional fields.
pub fn opt_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v: Option<Value> = serde::Deserialize::deserialize(d)?;
    Ok(v.filter(|v| !v.is_null()).map(|v| to_text(&v)))
}

/// YAML to JSON (for the API and the run plan).
pub fn to_json(v: &Value) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_style_text() {
        let y = |s: &str| -> Value { serde_yaml::from_str(s).unwrap() };
        assert_eq!(to_text(&y("3.0")), "3.0");
        assert_eq!(to_text(&y("0.24.0")), "0.24.0");
        assert_eq!(to_text(&y("98.0")), "98.0");
        assert_eq!(to_text(&y("0.5")), "0.5");
        assert_eq!(to_text(&y("150")), "150");
        assert_eq!(to_text(&y("true")), "true");
        assert_eq!(to_text(&y("''")), "");
        assert_eq!(to_text(&y("~")), "");
    }
}
