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
    pub title: Setting,
    pub description: Setting,
    pub keywords: Setting,
    pub contact: Setting,
    pub gps: Setting,
    pub camera_make: Setting,
    pub camera_model: Setting,
    pub lens: Setting,
    pub device_serial: Setting,
    pub camera_settings: Setting,
    pub image_id: Setting,
    pub rating: Setting,
    pub jpeg_thumbnail: Setting,
    pub mpf: Setting,
    pub flashpix: Setting,
    pub ducky: Setting,
    pub png_extension: Setting,
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
            title: Setting::Keep,
            description: Setting::Keep,
            keywords: Setting::Keep,
            contact: Setting::Remove,
            gps: Setting::Remove,
            camera_make: Setting::Remove,
            camera_model: Setting::Remove,
            lens: Setting::Remove,
            device_serial: Setting::Remove,
            camera_settings: Setting::Remove,
            image_id: Setting::Remove,
            rating: Setting::Keep,
            jpeg_thumbnail: Setting::Remove,
            mpf: Setting::Keep,
            flashpix: Setting::Remove,
            ducky: Setting::Remove,
            png_extension: Setting::Keep,
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


/** Load metaclean.ini next to the executable, unless --config was specified. */
pub fn load_config(executable: &Path, explicit: Option<&Path>) -> Result<Config, String> {
    if let Some(path) = explicit { return Config::load(path); }
    let ini = executable.parent().ok_or("cannot determine executable directory")?.join("metaclean.ini");
    if ini.is_file() { Config::load(&ini) }
    else if ini.exists() { Err(format!("configuration path is not a regular file: {}", ini.display())) }
    else { Ok(Config::default()) }
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
                && !matches!(key, "creator" | "copyright" | "software" | "comments" | "ai_generation" | "other_text"
                    | "title" | "description" | "keywords" | "contact" | "camera_make" | "camera_model"
                    | "lens" | "device_serial" | "image_id")
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
                "title" => config.title = value,
                "description" => config.description = value,
                "keywords" => config.keywords = value,
                "contact" => config.contact = value,
                "gps" => config.gps = value,
                "camera_make" => config.camera_make = value,
                "camera_model" => config.camera_model = value,
                "lens" => config.lens = value,
                "device_serial" => config.device_serial = value,
                "camera_settings" => config.camera_settings = value,
                "image_id" => config.image_id = value,
                "rating" => config.rating = value,
                "jpeg_thumbnail" => config.jpeg_thumbnail = value,
                "mpf" => config.mpf = value,
                "flashpix" => config.flashpix = value,
                "ducky" => config.ducky = value,
                "png_extension" => config.png_extension = value,
                "unknown" => config.unknown = value,
                _ => return Err(format!("INI line {line_number}: unknown option {key}")),
            }
        }
        // If we preserve full containers, they may still contain the original name.
        let has_replacement = [
            &config.creator, &config.copyright, &config.software, &config.comments,
            &config.ai_generation, &config.other_text, &config.title,
            &config.description, &config.keywords, &config.contact,
            &config.camera_make, &config.camera_model, &config.lens,
            &config.device_serial, &config.image_id,
        ].iter().any(|setting| setting.replacement().is_some());
        if has_replacement && (config.exif.keep() || config.xmp.keep() || config.iptc.keep()) {
            return Err("cannot combine text replacement with exif=true, xmp=true or iptc=true: original identifying data could remain".into());
        }
        Ok(config)
    }

    /// Classify well-known standard and widely used PNG text keywords.
    /// Unknown keywords retain the independent other_text policy.
    pub fn png_keyword(&self, keyword: &[u8]) -> &Setting {
        let keyword = String::from_utf8_lossy(keyword).to_ascii_lowercase();
        let key = keyword.trim();
        if matches!(key, "xml:com.adobe.xmp" | "raw profile type xmp" | "xmp") {
            return &self.xmp;
        }
        // ImageMagick/ExifTool/GIMP commonly embed binary metadata in PNG
        // zTXt/iTXt under these well-known legacy keywords. They must not
        // fall through to other_text=true and accidentally leak author/GPS.
        if matches!(key, "raw profile type exif" | "raw profile type app1") {
            return &self.exif;
        }
        if matches!(key, "raw profile type iptc" | "raw profile type 8bim") {
            return &self.iptc;
        }
        if matches!(key, "raw profile type icc" | "raw profile type icm") {
            return &self.color_profile;
        }
        if matches!(
            key, "parameters" | "prompt" | "negative prompt" | "workflow" | "seed"
                | "sampler" | "model" | "steps" | "cfg scale" | "generation data"
                | "generation parameters" | "invokeai_metadata" | "comfyui"
                | "generation prompt" | "ai prompt" | "positive prompt"
        ) || key.contains("stable diffusion") || key.starts_with("comfyui") {
            return &self.ai_generation;
        }
        match key {
            "author" | "artist" | "creator" | "by-line" | "byline" | "owner" | "creator name" => &self.creator,
            "copyright" | "license owner" | "rights holder" | "rights" => &self.copyright,
            "software" | "tool" | "encoder" | "processing software" => &self.software,
            "creation time" | "date:create" | "date:modify" | "timestamp" | "date"
                | "date created" | "date modified" | "datetime" | "date time"
                | "create-date" | "modify-date" => &self.timestamps,
            "comment" | "usercomment" | "user comment" | "notes" => &self.comments,
            "title" | "headline" | "object name" => &self.title,
            "description" | "caption" | "image description" | "disclaimer" | "warning"
                | "subject" | "xp subject" => &self.description,
            "keywords" | "keyword" | "tags" | "tag list" => &self.keywords,
            "contact" | "email" | "email address" | "website" | "url" | "phone"
                | "telephone" | "address" | "creator contact info" => &self.contact,
            "location" | "gps" | "latitude" | "longitude" | "altitude" | "city"
                | "state" | "country" | "country code" | "sublocation" => &self.gps,
            "make" | "manufacturer" | "camera maker" => &self.camera_make,
            "model" | "camera" | "camera model" | "device" | "source" => &self.camera_model,
            "lens" | "lens model" | "lens make" => &self.lens,
            "serial number" | "device serial" | "body serial number" | "lens serial number" => &self.device_serial,
            "iso" | "exposure" | "aperture" | "focal length" | "shutter speed"
                | "flash" | "white balance" | "capture settings" => &self.camera_settings,
            "image id" | "image unique id" | "document id" | "instance id" => &self.image_id,
            "rating" | "rating percent" => &self.rating,
            _ => &self.other_text,
        }
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
    fn expanded_png_keywords_are_classified() {
        let c = Config::default();
        assert_eq!(c.png_keyword(b"Source"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"GPS"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"Email"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"Raw profile type exif"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"Raw profile type iptc"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"Raw profile type 8bim"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"Raw profile type icm"), &Setting::Keep);
        assert_eq!(c.png_keyword(b"create-date"), &Setting::Remove);
        assert_eq!(c.png_keyword(b"Title"), &Setting::Keep);
        assert_eq!(c.png_keyword(b"Tags"), &Setting::Keep);
        let r = Config::parse("[keep]\ntitle=\"New title\"\ncamera_model=\"New device\"\n").unwrap();
        assert_eq!(r.title.replacement(), Some("New title"));
        assert_eq!(r.camera_model.replacement(), Some("New device"));
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
