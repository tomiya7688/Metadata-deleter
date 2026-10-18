use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// A metadata policy. Quoted strings are literal replacement values, even "true"/"false".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    Keep,
    Remove,
    Replace(String),
}

impl Setting {
    pub fn keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    pub fn replacement(&self) -> Option<&str> {
        match self {
            Self::Replace(text) => Some(text),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub exif: Setting,
    pub xmp: Setting,
    pub iptc: Setting,
    pub comments: Setting,
    pub creator: Setting,
    pub copyright: Setting,
    pub software: Setting,
    pub timestamps: Setting,
    pub ai_generation: Setting,
    pub provenance: Setting,
    pub color_profile: Setting,
    pub orientation: Setting,
    pub resolution: Setting,
    pub other_text: Setting,
    pub unknown: Setting,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exif: Setting::Remove,
            xmp: Setting::Remove,
            iptc: Setting::Remove,
            comments: Setting::Remove,
            creator: Setting::Remove,
            copyright: Setting::Remove,
            software: Setting::Remove,
            timestamps: Setting::Remove,
            ai_generation: Setting::Remove,
            provenance: Setting::Remove,
            color_profile: Setting::Keep,
            orientation: Setting::Keep,
            resolution: Setting::Keep,
            other_text: Setting::Keep,
            unknown: Setting::Keep,
        }
    }
}

pub const DEFAULT_INI: &str = include_str!("../metaclean.ini");

fn parse_value(value: &str, line: usize, key: &str) -> Result<Setting, String> {
    let value = value.trim_start();
    if value.starts_with('"') {
        let mut result = String::new();
        let mut chars = value[1..].char_indices();
        let mut closed = None;
        while let Some((offset, ch)) = chars.next() {
            match ch {
                '"' => {
                    closed = Some(offset + 2); // includes opening and closing quotes
                    break;
                }
                '\\' => {
                    let (_, escaped) = chars
                        .next()
                        .ok_or_else(|| format!("INI line {line}: unterminated escape in {key}"))?;
                    match escaped {
                        '\\' | '"' => result.push(escaped),
                        _ => return Err(format!("INI line {line}: unsupported escape \\{escaped} in {key}")),
                    }
                }
                _ => result.push(ch),
            }
        }
        let end = closed.ok_or_else(|| format!("INI line {line}: missing closing double quote for {key}"))?;
        let suffix = value[end..].trim_start();
        if !suffix.is_empty() && !suffix.starts_with(';') && !suffix.starts_with('#') {
            return Err(format!("INI line {line}: unexpected characters after quoted value for {key}"));
        }
        if result.chars().any(char::is_control) {
            return Err(format!("INI line {line}: replacement for {key} contains control characters"));
        }
        return Ok(Setting::Replace(result));
    }
    match value.split([';', '#']).next().unwrap_or("").trim() {
        "true" => Ok(Setting::Keep),
        "false" => Ok(Setting::Remove),
        _ => Err(format!("INI line {line}: {key} must be true, false or a double-quoted string")),
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read config {}: {e}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut config = Self::default();
        let mut in_keep = false;
        let mut saw_section = false;
        let mut seen = HashSet::new();
        for (i, line) in text.lines().enumerate() {
            let line_number = i + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len() - 1];
                if section != "keep" {
                    return Err(format!("INI line {line_number}: unknown section [{section}]"));
                }
                if saw_section {
                    return Err(format!("INI line {line_number}: duplicate [keep] section"));
                }
                in_keep = true;
                saw_section = true;
                continue;
            }
            if !in_keep {
                return Err(format!("INI line {line_number}: expected [keep] section"));
            }
            let (key, raw_value) = line
                .split_once('=')
                .ok_or_else(|| format!("INI line {line_number}: expected key = value"))?;
            let key = key.trim();
            if !seen.insert(key.to_string()) {
                return Err(format!("INI line {line_number}: duplicate option {key}"));
            }
            let value = parse_value(raw_value, line_number, key)?;
            // Binary containers and image display fields cannot be replaced by arbitrary strings.
            if value.replacement().is_some()
                && !matches!(key, "creator" | "copyright" | "software" | "comments" | "ai_generation" | "other_text")
            {
                return Err(format!("INI line {line_number}: {key} does not support text replacement"));
            }
            match key {
                "exif" => config.exif = value,
                "xmp" => config.xmp = value,
                "iptc" => config.iptc = value,
                "comments" => config.comments = value,
                "creator" => config.creator = value,
                "copyright" => config.copyright = value,
                "software" => config.software = value,
                "timestamps" => config.timestamps = value,
                "ai_generation" => config.ai_generation = value,
                "provenance" => config.provenance = value,
                "color_profile" => config.color_profile = value,
                "orientation" => config.orientation = value,
                "resolution" => config.resolution = value,
                "other_text" => config.other_text = value,
                "unknown" => config.unknown = value,
                _ => return Err(format!("INI line {line_number}: unknown option {key}")),
            }
        }
        // If we preserve full containers, they may still contain the original name.
        if ["exif", "xmp", "iptc"].iter().any(|name| match *name {
            "exif" => config.exif.keep(),
            "xmp" => config.xmp.keep(),
            _ => config.iptc.keep(),
        }) && ["creator", "copyright", "software", "comments", "ai_generation", "other_text"]
            .iter()
            .any(|name| match *name {
                "creator" => config.creator.replacement().is_some(),
                "copyright" => config.copyright.replacement().is_some(),
                "software" => config.software.replacement().is_some(),
                "comments" => config.comments.replacement().is_some(),
                "ai_generation" => config.ai_generation.replacement().is_some(),
                _ => config.other_text.replacement().is_some(),
            })
        {
            return Err("cannot use quoted replacements with exif=true, xmp=true or iptc=true: original private data could remain".into());
        }
        Ok(config)
    }

    /// Classify a PNG keyword without decoding its compressed text value.
    pub fn png_keyword(&self, keyword: &[u8]) -> &Setting {
        let keyword = String::from_utf8_lossy(keyword).to_ascii_lowercase();
        let key = keyword.trim();
        if matches!(key, "xml:com.adobe.xmp" | "raw profile type xmp" | "xmp") {
            return &self.xmp;
        }
        if matches!(
            key,
            "parameters" | "prompt" | "negative prompt" | "workflow" | "seed"
                | "sampler" | "model" | "steps" | "cfg scale" | "generation data"
                | "generation parameters" | "invokeai_metadata" | "comfyui"
        ) || key.contains("stable diffusion")
        {
            return &self.ai_generation;
        }
        if matches!(key, "author" | "artist" | "creator" | "by-line" | "owner") {
            return &self.creator;
        }
        if matches!(key, "copyright" | "license owner" | "rights holder") {
            return &self.copyright;
        }
        if matches!(key, "software" | "tool" | "encoder" | "source") {
            return &self.software;
        }
        if matches!(key, "creation time" | "date:create" | "date:modify" | "timestamp" | "date") {
            return &self.timestamps;
        }
        if matches!(key, "comment" | "description" | "usercomment" | "user comment") {
            return &self.comments;
        }
        &self.other_text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        assert_eq!(Config::parse(DEFAULT_INI).unwrap(), Config::default());
        let c = Config::parse("[keep]\nexif = true\nother_text = false ; note\n").unwrap();
        assert_eq!(c.exif, Setting::Keep);
        assert_eq!(c.other_text, Setting::Remove);
        assert_eq!(c.ai_generation, Setting::Remove);
    }

    #[test]
    fn quoted_text_and_escaping() {
        let c = Config::parse("[keep]\ncreator = \"tomiya; # \\\"T\\\" = true\" ; note\ncomments = \"false\"\n").unwrap();
        assert_eq!(c.creator, Setting::Replace("tomiya; # \"T\" = true".into()));
        assert_eq!(c.comments, Setting::Replace("false".into()));
        assert_eq!(Config::parse("[keep]\ncreator = \"\"\n").unwrap().creator, Setting::Replace(String::new()));
    }

    #[test]
    fn rejects_unquoted_or_unsupported_replacement() {
        for bad in [
            "exif = true",
            "[oops]\nexif=true",
            "[keep]\ncreator = tomiya",
            "[keep]\ncreator = 'tomiya'",
            "[keep]\ncreator = \"unclosed",
            "[keep]\ncreator = \"x\" junk",
            "[keep]\ncreator = \"x\\q\"",
            "[keep]\ncolor_profile = \"tomiya\"",
            "[keep]\nrandom = true",
            "[keep]\nexif=true\nexif=false",
            "[keep]\nexif=true\ncreator=\"tomiya\"",
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn known_png_tags() {
        let c = Config::default();
        assert_eq!(c.png_keyword(b"parameters"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"XML:com.adobe.xmp"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"Title"), &Setting::Keep);
    }
}
