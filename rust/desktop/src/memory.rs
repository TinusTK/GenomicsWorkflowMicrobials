//! Memory for the tools.
//!
//! The rule (the user's): 4 GB stay for the computer itself, the rest is for
//! the tools. On Windows the tools run in WSL, whose memory is the `memory=`
//! line under `[wsl2]` in the user's `.wslconfig`; on a Mac in Colima's Linux
//! machine (`memory:` in its `colima.yaml`); on Linux straight on the computer.
//! Settings shows the amount and changes it, so nobody edits a file by hand.
// each system uses its own part of this file
#![allow(dead_code)]

/// All of the computer's memory but 4 GB (at least 2).
pub fn default_gb(total: u32) -> u32 {
    total.saturating_sub(4).max(2)
}

fn section(line: &str) -> Option<String> {
    let l = line.trim();
    (l.starts_with('[') && l.ends_with(']')).then(|| l[1..l.len() - 1].trim().to_lowercase())
}

/// The value of `memory` when `line` sets it.
fn memory_value(line: &str) -> Option<&str> {
    let l = line.trim();
    if l.starts_with('#') || l.starts_with(';') {
        return None;
    }
    let (key, value) = l.split_once('=')?;
    key.trim().eq_ignore_ascii_case("memory").then(|| value.trim())
}

/// GB of memory WSL is given in this `.wslconfig` text; nothing when it sets none.
pub fn wsl_memory_gb(text: &str) -> Option<u32> {
    let mut in_wsl2 = false;
    for line in text.lines() {
        if let Some(name) = section(line) {
            in_wsl2 = name == "wsl2";
        } else if in_wsl2 {
            if let Some(v) = memory_value(line) {
                let v = v.to_lowercase();
                let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
                let n: f64 = digits.parse().ok()?;
                let gb = match v[digits.len()..].trim() {
                    "gb" | "g" => n,
                    "mb" | "m" => n / 1024.0,
                    "kb" | "k" => n / 1_048_576.0,
                    _ => n / 1_073_741_824.0,
                };
                return Some(gb.round() as u32).filter(|g| *g > 0);
            }
        }
    }
    None
}

/// This `.wslconfig` text with WSL's memory set to `gb`; every other line stays.
pub fn with_wsl_memory(text: &str, gb: u32) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let new = format!("memory={gb}GB");
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut in_wsl2 = false;
    let mut header = None;
    let mut found = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = section(line) {
            in_wsl2 = name == "wsl2";
            if in_wsl2 && header.is_none() {
                header = Some(i);
            }
        } else if in_wsl2 && found.is_none() && memory_value(line).is_some() {
            found = Some(i);
        }
    }
    match (found, header) {
        (Some(i), _) => lines[i] = new,
        (None, Some(i)) => lines.insert(i + 1, new),
        (None, None) => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[wsl2]".into());
            lines.push(new);
        }
    }
    lines.join(eol) + eol
}

/// GB of memory Colima's machine is set to in this `colima.yaml` text.
pub fn colima_memory_gb(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|l| l.strip_prefix("memory:"))
        .and_then(|v| v.split('#').next()?.trim().parse::<f64>().ok())
        .map(|g| g.round() as u32)
        .filter(|g| *g > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_gb_stay_for_the_computer() {
        assert_eq!(default_gb(32), 28);
        assert_eq!(default_gb(16), 12);
        assert_eq!(default_gb(4), 2);
    }

    #[test]
    fn wsl_memory_is_read() {
        assert_eq!(wsl_memory_gb("[wsl2]\nmemory=18GB\nswap=16GB\n"), Some(18));
        assert_eq!(wsl_memory_gb("[wsl2]\r\n Memory = 8192MB \r\n"), Some(8));
        // only under [wsl2], and not a line that is commented out
        assert_eq!(wsl_memory_gb("[general]\nmemory=4GB\n"), None);
        assert_eq!(wsl_memory_gb("[wsl2]\n# memory=4GB\nswap=0\n"), None);
        assert_eq!(wsl_memory_gb(""), None);
    }

    #[test]
    fn wsl_memory_is_set_and_the_rest_kept() {
        // a line that is there is replaced
        let text = "[wsl2]\nmemory=18GB\nswap=16GB\n\n[experimental]\nautoMemoryReclaim=gradual\n";
        assert_eq!(
            with_wsl_memory(text, 28),
            "[wsl2]\nmemory=28GB\nswap=16GB\n\n[experimental]\nautoMemoryReclaim=gradual\n"
        );
        // a section without the line gets it
        assert_eq!(with_wsl_memory("[wsl2]\nswap=0\n", 12), "[wsl2]\nmemory=12GB\nswap=0\n");
        // no section, no file
        assert_eq!(with_wsl_memory("[general]\nx=1", 12), "[general]\nx=1\n\n[wsl2]\nmemory=12GB\n");
        assert_eq!(with_wsl_memory("", 12), "[wsl2]\nmemory=12GB\n");
        // Windows line ends stay
        assert_eq!(with_wsl_memory("[wsl2]\r\nswap=0\r\n", 12), "[wsl2]\r\nmemory=12GB\r\nswap=0\r\n");
        // and what was written reads back
        assert_eq!(wsl_memory_gb(&with_wsl_memory(text, 28)), Some(28));
    }

    #[test]
    fn colima_memory_is_read() {
        assert_eq!(colima_memory_gb("cpu: 4\nmemory: 12\ndisk: 100\n"), Some(12));
        assert_eq!(colima_memory_gb("memory: 4 # GiB\n"), Some(4));
        assert_eq!(colima_memory_gb("cpu: 4\n"), None);
    }
}
