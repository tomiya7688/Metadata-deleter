use std::fs;
use std::path::Path;

/// `true` means preserve this metadata; `false` means remove it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub exif: bool,
    pub xmp: bool,
    pub iptc: bool,
    pub comments: bool,
    pub creator: bool,
    pub copyright: bool,
    pub software: bool,
    pub timestamps: bool,
    pub ai_generation: bool,
    pub provenance: bool,
    pub color_profile: bool,
    pub orientation: bool,
    pub resolution: bool,
    pub other_text: bool,
    pub unknown: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exif: false,
            xmp: false,
            iptc: false,
            comments: false,
            creator: false,
            copyright: false,
            software: false,
            timestamps: false,
            ai_generation: false,
            provenance: false,
            color_profile: true,
            orientation: true,
            resolution: true,
            other_text: true,
            unknown: true,
        }
    }
}

pub const DEFAULT_INI: &str = r#"; true = keep, false = remove.
; EXIF/XMP/IPTC are metadata CONTAINERS. When kept, all their contents are kept.
; When EXIF is removed, orientation and resolution may be reconstructed separately.
; Unknown chunks and unrecognized text may contain private information.
[keep]
exif = false
xmp = false
iptc = false
comments = false
creator = false
copyright = false
software = false
timestamps = false
ai_generation = false
provenance = false
color_profile = true
orientation = true
resolution = true
other_text = true
unknown = true
"#;

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
        let mut seen = std::collections::HashSet::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len() - 1];
                if section != "keep" {
                    return Err(format!("INI line {}: unknown section [{section}]", i + 1));
                }
                if saw_section {
                    return Err(format!("INI line {}: duplicate [keep] section", i + 1));
                }
                in_keep = true;
                saw_section = true;
                continue;
            }
            if !in_keep {
                return Err(format!("INI line {}: expected [keep] section", i + 1));
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("INI line {}: expected key = true|false", i + 1))?;
            let key = key.trim();
            let value = value.split([';', '#']).next().unwrap_or("").trim();
            let keep = match value {
                "true" => true,
                "false" => false,
                _ => return Err(format!("INI line {}: {key} must be true or false", i + 1)),
            };
            if !seen.insert(key.to_string()) {
                return Err(format!("INI line {}: duplicate option {key}", i + 1));
            }
            match key {
                "exif" => config.exif = keep,
                "xmp" => config.xmp = keep,
                "iptc" => config.iptc = keep,
                "comments" => config.comments = keep,
                "creator" => config.creator = keep,
                "copyright" => config.copyright = keep,
                "software" => config.software = keep,
                "timestamps" => config.timestamps = keep,
                "ai_generation" => config.ai_generation = keep,
                "provenance" => config.provenance = keep,
                "color_profile" => config.color_profile = keep,
                "orientation" => config.orientation = keep,
                "resolution" => config.resolution = keep,
                "other_text" => config.other_text = keep,
                "unknown" => config.unknown = keep,
                _ => return Err(format!("INI line {}: unknown option {key}", i + 1)),
            }
        }
        Ok(config)
    }

    /// Classify an uncompressed or compressed PNG text keyword without decoding text values.
    pub fn keep_png_keyword(&self, keyword: &[u8]) -> bool {
        let keyword = String::from_utf8_lossy(keyword).to_ascii_lowercase();
        let key = keyword.trim();
        if matches!(key, "xml:com.adobe.xmp" | "raw profile type xmp" | "xmp") {
            return self.xmp;
        }
        if matches!(
            key,
            "parameters" | "prompt" | "negative prompt" | "workflow" | "seed"
                | "sampler" | "model" | "steps" | "cfg scale" | "generation data"
                | "generation parameters" | "invokeai_metadata" | "comfyui"
        ) || key.contains("stable diffusion")
        {
            return self.ai_generation;
        }
        if matches!(key, "author" | "artist" | "creator" | "by-line" | "owner") {
            return self.creator;
        }
        if matches!(key, "copyright" | "license owner" | "rights holder") {
            return self.copyright;
        }
        if matches!(key, "software" | "tool" | "encoder" | "source") {
            return self.software;
        }
        if matches!(
            key,
            "creation time" | "date:create" | "date:modify" | "timestamp" | "date"
        ) {
            return self.timestamps;
        }
        if matches!(key, "comment" | "description" | "usercomment" | "user comment") {
            return self.comments;
        }
        self.other_text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let a = Config::parse(DEFAULT_INI).unwrap();
        assert_eq!(a, Config::default());
        let b = Config::parse("[keep]\nexif = true\nother_text = false ; note\n").unwrap();
        assert!(b.exif);
        assert!(!b.other_text);
        assert!(!b.ai_generation);
    }

    #[test]
    fn bad_ini_fails_closed() {
        for bad in [
            "exif = true",
            "[oops]\nexif=true",
            "[keep]\nexif = sometimes",
            "[keep]\nrandom = true",
            "[keep]\nexif=true\nexif=false",
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn known_png_tags() {
        let c = Config::default();
        assert!(!c.keep_png_keyword(b"parameters"));
        assert!(!c.keep_png_keyword(b"XML:com.adobe.xmp"));
        assert!(c.keep_png_keyword(b"Title"));
    }
}
